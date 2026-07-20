//! Stable, portable asset identities for the A37/XUI18 power-balance UI.
//!
//! Runtime A37 rows carry engine-selected sprite names. This adapter derives
//! authored roles from the authenticated effective GUI winner, then joins both
//! authored and engine-selected names to the effective GFX manifest. A binding
//! is therefore a game-relative GUI/GFX/texture identity plus exact payload
//! hash/length, never an install-local path, vanilla whitelist, or guessed
//! filename.

use crate::assets::AssetResolver;
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::power_balance::{PowerBalancePresentation, PowerBalanceRuntimeSupplement};
use replay_core::UsedContentKey;
use serde::Deserialize;
use serde_json::Value;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

const GFX_MANIFEST_SCHEMA: &str = "gfx_icon_asset_manifest_static.static_artifact.v2";
const GUI_ROLE_SCHEMA: &str = "gfx_icon_asset_manifest_static.power_balance_gui_roles.v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PowerBalanceAssetRole {
    PanelBackground,
    PanelArtwork,
    DecisionContainerFrame,
    HeaderBackground,
    CloseButton,
    DecisionBackground,
    BalanceFrame,
    BalanceValue,
    PositionMarker,
    RangeIndicator,
    RangeSplitter,
    LeftSideIcon,
    RightSideIcon,
    LauncherButton,
    LauncherGlow,
    LauncherIcon,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortablePowerBalanceAsset {
    /// Hash of the canonical static GFX manifest without its self-hash field.
    pub manifest_artifact_sha256: String,
    pub role: PowerBalanceAssetRole,
    pub gui_source_path: String,
    pub gui_source_layer: String,
    pub gui_source_layer_kind: String,
    pub gui_source_sha256: String,
    pub gui_source_bytes: u64,
    pub sprite_name: String,
    pub drawable_kind: String,
    pub source_gfx_path: String,
    pub source_layer: String,
    pub source_layer_kind: String,
    pub texture_slot: String,
    pub texture_path: String,
    pub texture_layer: String,
    pub texture_layer_kind: String,
    pub texture_sha256: String,
    pub texture_bytes: u64,
    /// Zero-based frame selected by A37. `None` means the authored sprite
    /// animation owns frame selection (currently only launcher glow).
    pub selected_frame: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerBalanceAssetSet {
    pub manifest_artifact_sha256: String,
    pub bindings: Vec<PortablePowerBalanceAsset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortablePowerBalanceContentAsset {
    pub role: PowerBalanceAssetRole,
    pub gui_source: UsedContentKey,
    pub sprite_name: String,
    pub drawable_kind: String,
    pub source_gfx: UsedContentKey,
    pub texture_slot: String,
    pub texture: UsedContentKey,
    pub selected_frame: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortablePowerBalanceContentSet {
    pub effective_vfs_manifest_sha256: String,
    /// Exact winning GUI definitions from which semantic roles were derived.
    pub gui_sources: Vec<UsedContentKey>,
    pub bindings: Vec<PortablePowerBalanceContentAsset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerBalanceAssetError {
    InvalidJson,
    UnsupportedSchema,
    InvalidManifestHash,
    InvalidGuiManifest,
    MissingGuiRole(PowerBalanceAssetRole),
    AmbiguousGuiRole(PowerBalanceAssetRole),
    DuplicateSprite(String),
    MissingSprite(String),
    InvalidSprite(String),
    MissingTextureIdentity(String),
    InvalidSelectedFrame(String),
    DuplicateRoleSlot(PowerBalanceAssetRole, String),
}

#[derive(Clone, Debug)]
pub struct PowerBalanceAssetCatalog {
    artifact_sha256: String,
    sprites: BTreeMap<String, ManifestSprite>,
    gui_roles: BTreeMap<PowerBalanceAssetRole, GuiRoleBinding>,
}

impl PowerBalanceAssetCatalog {
    pub fn from_static_manifest_json(input: &str) -> Result<Self, PowerBalanceAssetError> {
        let mut canonical: Value =
            serde_json::from_str(input).map_err(|_| PowerBalanceAssetError::InvalidJson)?;
        let object = canonical
            .as_object_mut()
            .ok_or(PowerBalanceAssetError::InvalidJson)?;
        let claimed = object
            .remove("artifact_sha256")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(PowerBalanceAssetError::InvalidManifestHash)?;
        if !is_sha256(&claimed)
            || Sha256::hex(
                &serde_json::to_vec(&canonical)
                    .map_err(|_| PowerBalanceAssetError::InvalidJson)?,
            ) != claimed
        {
            return Err(PowerBalanceAssetError::InvalidManifestHash);
        }

        let raw: RawManifest =
            serde_json::from_str(input).map_err(|_| PowerBalanceAssetError::InvalidJson)?;
        if raw.schema != GFX_MANIFEST_SCHEMA {
            return Err(PowerBalanceAssetError::UnsupportedSchema);
        }
        let mut sprites = BTreeMap::new();
        for (key, sprite) in raw.sprites {
            if key.trim().is_empty()
                || key != sprite.name
                || sprite.drawable_kind.trim().is_empty()
                || !is_relative(&sprite.source_file)
                || sprite.source_layer.trim().is_empty()
                || sprite.source_layer_kind.trim().is_empty()
            {
                return Err(PowerBalanceAssetError::InvalidSprite(key));
            }
            let folded = key.to_ascii_lowercase();
            if sprites.insert(folded, sprite).is_some() {
                return Err(PowerBalanceAssetError::DuplicateSprite(key));
            }
        }
        let gui_roles = validate_static_gui_manifest(raw.power_balance_gui)?;
        Ok(Self {
            artifact_sha256: raw.artifact_sha256,
            sprites,
            gui_roles,
        })
    }

    pub fn resolve(
        &self,
        panel: &PowerBalancePresentation,
    ) -> Result<PowerBalanceAssetSet, PowerBalanceAssetError> {
        validate_runtime_selection(
            &panel.bar_sprite,
            panel.launcher_visible,
            panel.launcher_icon_sprite.as_deref(),
        )?;
        let wanted = wanted_roles_from_manifest(
            &self.gui_roles,
            &panel.bar_sprite,
            panel.range_indicator_frame,
            &panel.left_icon,
            &panel.right_icon,
            panel.launcher_visible,
            panel.launcher_icon_sprite.as_deref(),
        )?;

        let mut bindings = Vec::new();
        let mut role_slots = BTreeSet::new();
        for (role, binding, sprite_name, selected_frame) in wanted {
            let sprite = self
                .sprites
                .get(&sprite_name.to_ascii_lowercase())
                .ok_or_else(|| PowerBalanceAssetError::MissingSprite(sprite_name.clone()))?;
            let frame_count = parse_frames(sprite.frames.as_ref())
                .ok_or_else(|| PowerBalanceAssetError::InvalidSprite(sprite_name.clone()))?;
            if selected_frame.is_some_and(|frame| frame >= frame_count) {
                return Err(PowerBalanceAssetError::InvalidSelectedFrame(
                    sprite_name.clone(),
                ));
            }
            let slots = sprite.normalized_slots();
            if slots.is_empty()
                || (role == PowerBalanceAssetRole::BalanceValue
                    && (!sprite.drawable_kind.eq_ignore_ascii_case("progressBarType")
                        || slots.len() != 2
                        || slots[0].slot.to_ascii_lowercase() != "texturefile1"
                        || slots[1].slot.to_ascii_lowercase() != "texturefile2"))
                || (role == PowerBalanceAssetRole::LauncherGlow
                    && !sprite
                        .drawable_kind
                        .eq_ignore_ascii_case("frameAnimatedSpriteType"))
            {
                return Err(PowerBalanceAssetError::InvalidSprite(
                    sprite_name.clone(),
                ));
            }
            if slots.is_empty() {
                return Err(PowerBalanceAssetError::MissingTextureIdentity(
                    sprite_name.clone(),
                ));
            }
            for slot in slots {
                let Some(texture_path) = slot.resolved_texturefile.as_deref() else {
                    return Err(PowerBalanceAssetError::MissingTextureIdentity(format!(
                        "{}:{}",
                        sprite_name, slot.slot
                    )));
                };
                let Some(texture_layer) = slot.texture_layer.as_deref() else {
                    return Err(PowerBalanceAssetError::MissingTextureIdentity(format!(
                        "{}:{}",
                        sprite_name, slot.slot
                    )));
                };
                let Some(texture_layer_kind) = slot.texture_layer_kind.as_deref() else {
                    return Err(PowerBalanceAssetError::MissingTextureIdentity(format!(
                        "{}:{}",
                        sprite_name, slot.slot
                    )));
                };
                let Some(texture_bytes) = slot.texture_bytes else {
                    return Err(PowerBalanceAssetError::MissingTextureIdentity(format!(
                        "{}:{}",
                        sprite_name, slot.slot
                    )));
                };
                let Some(texture_sha256) = slot.texture_sha256.as_deref() else {
                    return Err(PowerBalanceAssetError::MissingTextureIdentity(format!(
                        "{}:{}",
                        sprite_name, slot.slot
                    )));
                };
                if slot.slot.trim().is_empty()
                    || !is_relative(texture_path)
                    || texture_layer.trim().is_empty()
                    || texture_layer_kind.trim().is_empty()
                    || texture_bytes == 0
                    || !is_sha256(texture_sha256)
                {
                    return Err(PowerBalanceAssetError::MissingTextureIdentity(format!(
                        "{}:{}",
                        sprite_name, slot.slot
                    )));
                }
                let slot_key = slot.slot.to_ascii_lowercase();
                if !role_slots.insert((role, slot_key)) {
                    return Err(PowerBalanceAssetError::DuplicateRoleSlot(
                        role,
                        slot.slot.clone(),
                    ));
                }
                bindings.push(PortablePowerBalanceAsset {
                    manifest_artifact_sha256: self.artifact_sha256.clone(),
                    role,
                    gui_source_path: binding.source_file.clone(),
                    gui_source_layer: binding.source.source_layer.clone(),
                    gui_source_layer_kind: binding.source.source_layer_kind.clone(),
                    gui_source_sha256: binding.source.sha256.clone(),
                    gui_source_bytes: binding.source.bytes,
                    sprite_name: sprite.name.clone(),
                    drawable_kind: sprite.drawable_kind.clone(),
                    source_gfx_path: normalize(&sprite.source_file),
                    source_layer: sprite.source_layer.clone(),
                    source_layer_kind: sprite.source_layer_kind.clone(),
                    texture_slot: slot.slot,
                    texture_path: normalize(texture_path),
                    texture_layer: texture_layer.to_string(),
                    texture_layer_kind: texture_layer_kind.to_string(),
                    texture_sha256: texture_sha256.to_string(),
                    texture_bytes,
                    selected_frame,
                });
            }
        }
        Ok(PowerBalanceAssetSet {
            manifest_artifact_sha256: self.artifact_sha256.clone(),
            bindings,
        })
    }
}

/// Resolve the same roles directly against the selected-Present resolver.
/// Every returned source/texture key is already bound to the replay's exact
/// effective-VFS manifest, making this the production adapter; the static
/// manifest adapter above is retained for offline evidence and preflight.
pub fn resolve_power_balance_content_assets(
    runtime: &PowerBalanceRuntimeSupplement,
    registry: &SpriteRegistry,
    resolver: &AssetResolver,
) -> Result<PortablePowerBalanceContentSet, PowerBalanceAssetError> {
    validate_runtime_selection(
        runtime.bar_state.sprite(),
        runtime.launcher_visible,
        runtime.launcher_icon_sprite.as_deref(),
    )?;
    let (gui_roles, gui_sources) = load_admitted_gui_roles(resolver, runtime.launcher_visible)?;
    let wanted = wanted_roles_from_manifest(
        &gui_roles,
        runtime.bar_state.sprite(),
        runtime.range_indicator_frame,
        &runtime.effective_left_icon,
        &runtime.effective_right_icon,
        runtime.launcher_visible,
        runtime.launcher_icon_sprite.as_deref(),
    )?;
    let mut bindings = Vec::new();
    let mut role_slots = BTreeSet::new();
    let mut authority = None;
    for source in &gui_sources {
        match authority.as_ref() {
            Some(existing) if existing != &source.effective_vfs_manifest_sha256 => {
                return Err(PowerBalanceAssetError::InvalidManifestHash);
            }
            None => authority = Some(source.effective_vfs_manifest_sha256.clone()),
            _ => {}
        }
    }
    for (role, binding, sprite_name, selected_frame) in wanted {
        let gui_source = gui_sources
            .iter()
            .find(|source| {
                normalize(&source.normalized_game_relative_path)
                    .eq_ignore_ascii_case(&binding.source_file)
            })
            .cloned()
            .ok_or(PowerBalanceAssetError::InvalidGuiManifest)?;
        let sprite = registry
            .get(&sprite_name)
            .ok_or_else(|| PowerBalanceAssetError::MissingSprite(sprite_name.clone()))?;
        let frame_count = sprite.no_of_frames;
        if selected_frame.is_some_and(|frame| frame >= frame_count) {
            return Err(PowerBalanceAssetError::InvalidSelectedFrame(sprite_name));
        }
        if sprite.texture_files.is_empty()
            || (role == PowerBalanceAssetRole::BalanceValue
                && (!sprite.kind.eq_ignore_ascii_case("progressBarType")
                    || sprite.texture_files.len() != 2
                    || sprite.texture_files[0].key.to_ascii_lowercase() != "texturefile1"
                    || sprite.texture_files[1].key.to_ascii_lowercase() != "texturefile2"))
            || (role == PowerBalanceAssetRole::LauncherGlow
                && !sprite.kind.eq_ignore_ascii_case("frameAnimatedSpriteType"))
        {
            return Err(PowerBalanceAssetError::InvalidSprite(sprite_name));
        }
        let source_gfx = resolver
            .admitted_content_key(&sprite.source_file)
            .cloned()
            .ok_or_else(|| {
                PowerBalanceAssetError::MissingTextureIdentity(format!(
                    "{}:source_gfx",
                    sprite.name
                ))
            })?;
        source_gfx
            .validate()
            .map_err(|_| PowerBalanceAssetError::InvalidSprite(sprite.name.clone()))?;
        match authority.as_ref() {
            Some(existing) if existing != &source_gfx.effective_vfs_manifest_sha256 => {
                return Err(PowerBalanceAssetError::InvalidManifestHash);
            }
            None => authority = Some(source_gfx.effective_vfs_manifest_sha256.clone()),
            _ => {}
        }
        for slot in &sprite.texture_files {
            let slot_key = slot.key.to_ascii_lowercase();
            if !role_slots.insert((role, slot_key)) {
                return Err(PowerBalanceAssetError::DuplicateRoleSlot(
                    role,
                    slot.key.clone(),
                ));
            }
            let texture = resolver
                .admitted_content_key(&normalize(&slot.path))
                .cloned()
                .ok_or_else(|| {
                    PowerBalanceAssetError::MissingTextureIdentity(format!(
                        "{}:{}",
                        sprite.name, slot.key
                    ))
                })?;
            texture.validate().map_err(|_| {
                PowerBalanceAssetError::MissingTextureIdentity(format!(
                    "{}:{}",
                    sprite.name, slot.key
                ))
            })?;
            if texture.effective_vfs_manifest_sha256
                != source_gfx.effective_vfs_manifest_sha256
            {
                return Err(PowerBalanceAssetError::InvalidManifestHash);
            }
            bindings.push(PortablePowerBalanceContentAsset {
                role,
                gui_source: gui_source.clone(),
                sprite_name: sprite.name.clone(),
                drawable_kind: sprite.kind.clone(),
                source_gfx: source_gfx.clone(),
                texture_slot: slot.key.clone(),
                texture,
                selected_frame,
            });
        }
    }
    Ok(PortablePowerBalanceContentSet {
        effective_vfs_manifest_sha256: authority
            .ok_or(PowerBalanceAssetError::InvalidManifestHash)?,
        gui_sources,
        bindings,
    })
}

fn validate_runtime_selection(
    bar_sprite: &str,
    launcher_visible: bool,
    launcher_icon: Option<&str>,
) -> Result<(), PowerBalanceAssetError> {
    if bar_sprite.trim().is_empty() {
        return Err(PowerBalanceAssetError::InvalidSprite(
            bar_sprite.to_string(),
        ));
    }
    if launcher_visible && launcher_icon.is_none_or(str::is_empty) {
        return Err(PowerBalanceAssetError::InvalidSprite(
            "launcher_icon_sprite".into(),
        ));
    }
    Ok(())
}

fn wanted_roles_from_manifest<'a>(
    roles: &'a BTreeMap<PowerBalanceAssetRole, GuiRoleBinding>,
    bar_sprite: &str,
    range_indicator_frame: usize,
    left_icon: &str,
    right_icon: &str,
    launcher_visible: bool,
    launcher_icon: Option<&str>,
) -> Result<Vec<(PowerBalanceAssetRole, &'a GuiRoleBinding, String, Option<u32>)>, PowerBalanceAssetError> {
    let mut wanted = Vec::new();
    for role in [
        PowerBalanceAssetRole::PanelBackground,
        PowerBalanceAssetRole::PanelArtwork,
        PowerBalanceAssetRole::DecisionContainerFrame,
        PowerBalanceAssetRole::HeaderBackground,
        PowerBalanceAssetRole::CloseButton,
        PowerBalanceAssetRole::DecisionBackground,
        PowerBalanceAssetRole::BalanceFrame,
        PowerBalanceAssetRole::BalanceValue,
        PowerBalanceAssetRole::PositionMarker,
        PowerBalanceAssetRole::RangeIndicator,
        PowerBalanceAssetRole::RangeSplitter,
        PowerBalanceAssetRole::LeftSideIcon,
        PowerBalanceAssetRole::RightSideIcon,
    ] {
        let binding = roles
            .get(&role)
            .ok_or(PowerBalanceAssetError::MissingGuiRole(role))?;
        let sprite = match role {
            PowerBalanceAssetRole::BalanceValue => bar_sprite,
            PowerBalanceAssetRole::LeftSideIcon => left_icon,
            PowerBalanceAssetRole::RightSideIcon => right_icon,
            _ => binding
                .sprite_name
                .as_deref()
                .ok_or(PowerBalanceAssetError::MissingGuiRole(role))?,
        };
        let frame = match role {
            PowerBalanceAssetRole::RangeIndicator => Some(
                u32::try_from(range_indicator_frame).map_err(|_| {
                    PowerBalanceAssetError::InvalidSelectedFrame(sprite.to_string())
                })?,
            ),
            _ => Some(0),
        };
        wanted.push((role, binding, sprite.to_string(), frame));
    }
    if launcher_visible {
        for role in [
            PowerBalanceAssetRole::LauncherButton,
            PowerBalanceAssetRole::LauncherGlow,
            PowerBalanceAssetRole::LauncherIcon,
        ] {
            let binding = roles
                .get(&role)
                .ok_or(PowerBalanceAssetError::MissingGuiRole(role))?;
            let sprite = if role == PowerBalanceAssetRole::LauncherIcon {
                launcher_icon.ok_or(PowerBalanceAssetError::MissingGuiRole(role))?
            } else {
                binding
                    .sprite_name
                    .as_deref()
                    .ok_or(PowerBalanceAssetError::MissingGuiRole(role))?
            };
            let frame = (role != PowerBalanceAssetRole::LauncherGlow).then_some(0);
            wanted.push((role, binding, sprite.to_string(), frame));
        }
    }
    Ok(wanted)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GuiBindingSource {
    Authored,
    AuthoredAnimation,
    Engine,
}

#[derive(Clone, Debug)]
struct GuiSourceIdentity {
    source_layer: String,
    source_layer_kind: String,
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Debug)]
struct GuiRoleBinding {
    source_file: String,
    source: GuiSourceIdentity,
    sprite_name: Option<String>,
}

#[derive(Clone, Copy)]
struct GuiRoleSpec {
    role: PowerBalanceAssetRole,
    source_file: &'static str,
    hierarchy_suffix: &'static [&'static str],
    widget_type: &'static str,
    binding_source: GuiBindingSource,
}

const GUI_ROLE_SPECS: &[GuiRoleSpec] = &[
    GuiRoleSpec { role: PowerBalanceAssetRole::PanelBackground, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "background"], widget_type: "background", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::PanelArtwork, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "bop_bg"], widget_type: "iconType", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::DecisionContainerFrame, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "decision_container", "Background"], widget_type: "background", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::HeaderBackground, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "power_balance_header_bg"], widget_type: "iconType", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::CloseButton, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "close"], widget_type: "buttonType", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::DecisionBackground, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "Background"], widget_type: "background", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::BalanceFrame, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "power_balance_frame"], widget_type: "iconType", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::BalanceValue, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "power_balance_value"], widget_type: "iconType", binding_source: GuiBindingSource::Engine },
    GuiRoleSpec { role: PowerBalanceAssetRole::PositionMarker, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "position_marker"], widget_type: "iconType", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::RangeIndicator, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["range_indicator"], widget_type: "iconType", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::RangeSplitter, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["range_bar"], widget_type: "iconType", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::LeftSideIcon, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "left_power_icon"], widget_type: "iconType", binding_source: GuiBindingSource::Engine },
    GuiRoleSpec { role: PowerBalanceAssetRole::RightSideIcon, source_file: "interface/powerbalanceview.gui", hierarchy_suffix: &["powerbalanceview", "right_power_icon"], widget_type: "iconType", binding_source: GuiBindingSource::Engine },
    GuiRoleSpec { role: PowerBalanceAssetRole::LauncherButton, source_file: "interface/countrypoliticsview.gui", hierarchy_suffix: &["power_balance_button", "show_power_balance"], widget_type: "buttonType", binding_source: GuiBindingSource::Authored },
    GuiRoleSpec { role: PowerBalanceAssetRole::LauncherGlow, source_file: "interface/countrypoliticsview.gui", hierarchy_suffix: &["power_balance_button", "show_power_balance_glow"], widget_type: "iconType", binding_source: GuiBindingSource::AuthoredAnimation },
    GuiRoleSpec { role: PowerBalanceAssetRole::LauncherIcon, source_file: "interface/countrypoliticsview.gui", hierarchy_suffix: &["power_balance_button", "power_balance_ICON"], widget_type: "iconType", binding_source: GuiBindingSource::Engine },
];

fn load_admitted_gui_roles(
    resolver: &AssetResolver,
    include_launcher: bool,
) -> Result<(BTreeMap<PowerBalanceAssetRole, GuiRoleBinding>, Vec<UsedContentKey>), PowerBalanceAssetError> {
    let mut layouts = BTreeMap::new();
    let mut sources = BTreeMap::new();
    for source_file in ["interface/powerbalanceview.gui", "interface/countrypoliticsview.gui"] {
        if source_file.ends_with("countrypoliticsview.gui") && !include_launcher {
            continue;
        }
        let key = resolver
            .admitted_content_key(source_file)
            .cloned()
            .ok_or(PowerBalanceAssetError::InvalidGuiManifest)?;
        key.validate().map_err(|_| PowerBalanceAssetError::InvalidGuiManifest)?;
        if !normalize(&key.normalized_game_relative_path).eq_ignore_ascii_case(source_file) {
            return Err(PowerBalanceAssetError::InvalidGuiManifest);
        }
        let file = resolver
            .resolve_file(source_file)
            .map_err(|_| PowerBalanceAssetError::InvalidGuiManifest)?;
        let payload = std::fs::read(&file.path).map_err(|_| PowerBalanceAssetError::InvalidGuiManifest)?;
        if payload.len() as u64 != key.payload_bytes || Sha256::hex(&payload) != key.payload_sha256 {
            return Err(PowerBalanceAssetError::InvalidGuiManifest);
        }
        let mut layout = LayoutSet::default();
        layout
            .parse_file(&file.path, source_file)
            .map_err(|_| PowerBalanceAssetError::InvalidGuiManifest)?;
        if layout.documents.len() != 1 || !layout.documents[0].recoveries.is_empty() {
            return Err(PowerBalanceAssetError::InvalidGuiManifest);
        }
        layouts.insert(source_file, layout);
        sources.insert(source_file, key);
    }

    let mut roles = BTreeMap::new();
    for spec in GUI_ROLE_SPECS {
        if spec.source_file.ends_with("countrypoliticsview.gui") && !include_launcher {
            continue;
        }
        let layout = layouts
            .get(spec.source_file)
            .ok_or(PowerBalanceAssetError::InvalidGuiManifest)?;
        let mut matches = Vec::new();
        collect_gui_role_matches(
            &layout.documents[0].roots,
            &[],
            spec,
            &mut matches,
        );
        if matches.is_empty() {
            return Err(PowerBalanceAssetError::MissingGuiRole(spec.role));
        }
        if matches.len() != 1 {
            return Err(PowerBalanceAssetError::AmbiguousGuiRole(spec.role));
        }
        let (element, _widget_type_ordinal) = matches[0];
        let sprite_name = match spec.binding_source {
            GuiBindingSource::Authored | GuiBindingSource::AuthoredAnimation => {
                if element.sprite_refs.len() != 1 {
                    return Err(PowerBalanceAssetError::AmbiguousGuiRole(spec.role));
                }
                Some(element.sprite_refs[0].clone())
            }
            GuiBindingSource::Engine => {
                if spec.role != PowerBalanceAssetRole::LauncherIcon && !element.sprite_refs.is_empty() {
                    return Err(PowerBalanceAssetError::InvalidGuiManifest);
                }
                None
            }
        };
        let key = sources
            .get(spec.source_file)
            .ok_or(PowerBalanceAssetError::InvalidGuiManifest)?;
        roles.insert(spec.role, GuiRoleBinding {
            source_file: spec.source_file.to_string(),
            source: GuiSourceIdentity {
                source_layer: key.layer_id.clone(),
                source_layer_kind: format!("{:?}", key.layer_kind).to_ascii_lowercase(),
                sha256: key.payload_sha256.clone(),
                bytes: key.payload_bytes,
            },
            sprite_name,
        });
    }
    Ok((roles, sources.into_values().collect()))
}

fn collect_gui_role_matches<'a>(
    elements: &'a [GuiElement],
    ancestors: &[&'a str],
    spec: &GuiRoleSpec,
    out: &mut Vec<(&'a GuiElement, usize)>,
) {
    let mut type_ordinals = BTreeMap::<String, usize>::new();
    for element in elements {
        let type_ordinal = type_ordinals
            .entry(element.element_type.to_ascii_lowercase())
            .and_modify(|ordinal| *ordinal += 1)
            .or_insert(0);
        let current_ordinal = *type_ordinal;
        let mut path = ancestors.to_vec();
        path.push(&element.name);
        if element.element_type.eq_ignore_ascii_case(spec.widget_type)
            && path.len() >= spec.hierarchy_suffix.len()
            // GUI names are exact authored identities.  The a729
            // powerbalanceview deliberately has direct-child `background`
            // and `Background` widgets for different roles.
            && path[path.len() - spec.hierarchy_suffix.len()..]
                .iter()
                .zip(spec.hierarchy_suffix)
                .all(|(actual, expected)| actual == expected)
        {
            out.push((element, current_ordinal));
        }
        collect_gui_role_matches(&element.children, &path, spec, out);
    }
}

#[derive(Deserialize)]
struct RawManifest {
    schema: String,
    artifact_sha256: String,
    power_balance_gui: RawPowerBalanceGuiManifest,
    #[serde(deserialize_with = "deserialize_sprite_entries")]
    sprites: Vec<(String, ManifestSprite)>,
}

#[derive(Deserialize)]
struct RawPowerBalanceGuiManifest {
    schema: String,
    sources: BTreeMap<String, RawGuiSource>,
    roles: BTreeMap<String, RawGuiRole>,
}

#[derive(Clone, Deserialize)]
struct RawGuiSource {
    source_layer: String,
    source_layer_kind: String,
    bytes: u64,
    sha256: String,
}

#[derive(Deserialize)]
struct RawGuiRole {
    source_file: String,
    source_layer: String,
    source_layer_kind: String,
    hierarchy_path: String,
    parent_hierarchy_path: String,
    widget_type: String,
    widget_type_ordinal: usize,
    widget_name: String,
    hierarchy_match: String,
    binding_source: String,
    #[serde(default)]
    authored_sprite_fields: Vec<RawAuthoredSprite>,
    sprite_name: Option<String>,
}

#[derive(Deserialize)]
struct RawAuthoredSprite {
    field: String,
    sprite_name: String,
}

fn validate_static_gui_manifest(
    raw: RawPowerBalanceGuiManifest,
) -> Result<BTreeMap<PowerBalanceAssetRole, GuiRoleBinding>, PowerBalanceAssetError> {
    if raw.schema != GUI_ROLE_SCHEMA
        || raw.roles.len() != GUI_ROLE_SPECS.len()
        || raw.sources.len() != 2
    {
        return Err(PowerBalanceAssetError::InvalidGuiManifest);
    }
    for (path, source) in &raw.sources {
        if !is_relative(path)
            || source.source_layer.trim().is_empty()
            || source.source_layer_kind.trim().is_empty()
            || source.bytes == 0
            || !is_sha256(&source.sha256)
        {
            return Err(PowerBalanceAssetError::InvalidGuiManifest);
        }
    }
    let mut roles = BTreeMap::new();
    let mut structural_selectors = BTreeSet::new();
    for spec in GUI_ROLE_SPECS {
        let row = raw
            .roles
            .get(spec.role.manifest_key())
            .ok_or(PowerBalanceAssetError::MissingGuiRole(spec.role))?;
        let source = raw
            .sources
            .get(&row.source_file)
            .ok_or(PowerBalanceAssetError::InvalidGuiManifest)?;
        let hierarchy = row.hierarchy_path.split('/').collect::<Vec<_>>();
        let parent_hierarchy = hierarchy[..hierarchy.len().saturating_sub(1)].join("/");
        if !row.source_file.eq_ignore_ascii_case(spec.source_file)
            || row.source_layer != source.source_layer
            || row.source_layer_kind != source.source_layer_kind
            || !row.widget_type.eq_ignore_ascii_case(spec.widget_type)
            || row.widget_name.trim().is_empty()
            || hierarchy.last().is_none_or(|name| *name != row.widget_name)
            || row.parent_hierarchy_path != parent_hierarchy
            || row.hierarchy_match != "exact_case_sensitive"
            || hierarchy.len() < spec.hierarchy_suffix.len()
            || !hierarchy[hierarchy.len() - spec.hierarchy_suffix.len()..]
                .iter()
                .zip(spec.hierarchy_suffix)
                .all(|(actual, expected)| actual == expected)
        {
            return Err(PowerBalanceAssetError::InvalidGuiManifest);
        }
        if !structural_selectors.insert((
            row.source_file.to_ascii_lowercase(),
            row.parent_hierarchy_path.clone(),
            row.widget_type.to_ascii_lowercase(),
            row.widget_type_ordinal,
        )) {
            return Err(PowerBalanceAssetError::InvalidGuiManifest);
        }
        let binding_source = match row.binding_source.as_str() {
            "authored" => GuiBindingSource::Authored,
            "authored_animation" => GuiBindingSource::AuthoredAnimation,
            "engine" => GuiBindingSource::Engine,
            _ => return Err(PowerBalanceAssetError::InvalidGuiManifest),
        };
        if binding_source != spec.binding_source {
            return Err(PowerBalanceAssetError::InvalidGuiManifest);
        }
        let sprite_name = match binding_source {
            GuiBindingSource::Authored | GuiBindingSource::AuthoredAnimation => {
                if row.authored_sprite_fields.len() != 1
                    || row.authored_sprite_fields[0].field.trim().is_empty()
                    || row.authored_sprite_fields[0].sprite_name.trim().is_empty()
                    || row.sprite_name.as_deref()
                        != Some(row.authored_sprite_fields[0].sprite_name.as_str())
                {
                    return Err(PowerBalanceAssetError::InvalidGuiManifest);
                }
                row.sprite_name.clone()
            }
            GuiBindingSource::Engine => {
                if row.sprite_name.is_some()
                    || (spec.role != PowerBalanceAssetRole::LauncherIcon
                        && !row.authored_sprite_fields.is_empty())
                {
                    return Err(PowerBalanceAssetError::InvalidGuiManifest);
                }
                None
            }
        };
        roles.insert(spec.role, GuiRoleBinding {
            source_file: row.source_file.clone(),
            source: GuiSourceIdentity {
                source_layer: source.source_layer.clone(),
                source_layer_kind: source.source_layer_kind.clone(),
                sha256: source.sha256.clone(),
                bytes: source.bytes,
            },
            sprite_name,
        });
    }
    Ok(roles)
}

impl PowerBalanceAssetRole {
    fn manifest_key(self) -> &'static str {
        match self {
            Self::PanelBackground => "panel_background",
            Self::PanelArtwork => "panel_artwork",
            Self::DecisionContainerFrame => "decision_container_frame",
            Self::HeaderBackground => "header_background",
            Self::CloseButton => "close_button",
            Self::DecisionBackground => "decision_background",
            Self::BalanceFrame => "balance_frame",
            Self::BalanceValue => "balance_value",
            Self::PositionMarker => "position_marker",
            Self::RangeIndicator => "range_indicator",
            Self::RangeSplitter => "range_splitter",
            Self::LeftSideIcon => "left_side_icon",
            Self::RightSideIcon => "right_side_icon",
            Self::LauncherButton => "launcher_button",
            Self::LauncherGlow => "launcher_glow",
            Self::LauncherIcon => "launcher_icon",
        }
    }
}

fn deserialize_sprite_entries<'de, D>(
    deserializer: D,
) -> Result<Vec<(String, ManifestSprite)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<(String, ManifestSprite)>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map of GFX sprite definitions")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut entries = Vec::with_capacity(access.size_hint().unwrap_or(0));
            while let Some(entry) = access.next_entry::<String, ManifestSprite>()? {
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_map(Visitor)
}

#[derive(Clone, Debug, Deserialize)]
struct ManifestSprite {
    name: String,
    #[serde(default = "default_drawable_kind")]
    drawable_kind: String,
    source_file: String,
    source_layer: String,
    source_layer_kind: String,
    frames: Option<Value>,
    #[serde(default)]
    texture_slots: Vec<ManifestTextureSlot>,
    resolved_texturefile: Option<String>,
    texture_layer: Option<String>,
    texture_layer_kind: Option<String>,
    texture_bytes: Option<u64>,
    texture_sha256: Option<String>,
}

impl ManifestSprite {
    fn normalized_slots(&self) -> Vec<ManifestTextureSlot> {
        if !self.texture_slots.is_empty() {
            return self.texture_slots.clone();
        }
        match (
            self.resolved_texturefile.clone(),
            self.texture_layer.clone(),
            self.texture_layer_kind.clone(),
            self.texture_bytes,
            self.texture_sha256.clone(),
        ) {
            (Some(path), Some(layer), Some(kind), Some(bytes), Some(hash)) => {
                vec![ManifestTextureSlot {
                    slot: "texturefile".into(),
                    resolved_texturefile: Some(path),
                    texture_layer: Some(layer),
                    texture_layer_kind: Some(kind),
                    texture_bytes: Some(bytes),
                    texture_sha256: Some(hash),
                }]
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ManifestTextureSlot {
    slot: String,
    resolved_texturefile: Option<String>,
    texture_layer: Option<String>,
    texture_layer_kind: Option<String>,
    texture_bytes: Option<u64>,
    texture_sha256: Option<String>,
}

fn default_drawable_kind() -> String {
    "spriteType".into()
}

fn parse_frames(value: Option<&Value>) -> Option<u32> {
    match value {
        None | Some(Value::Null) => Some(1),
        Some(Value::String(value)) => value.parse::<u32>().ok().filter(|value| *value > 0),
        Some(Value::Number(value)) => value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0),
        _ => None,
    }
}

fn normalize(value: &str) -> String {
    let mut value = value.replace('\\', "/");
    while value.contains("//") {
        value = value.replace("//", "/");
    }
    value.trim_start_matches('/').to_string()
}

fn is_relative(value: &str) -> bool {
    let normalized = normalize(value);
    !normalized.is_empty()
        && !normalized.starts_with("../")
        && !normalized.contains("/../")
        && normalized.get(1..3) != Some(":/")
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
