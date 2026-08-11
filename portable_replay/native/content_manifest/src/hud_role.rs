//! Neutral, pointer-free HUD role wire schema shared by capture and replay.

use crate::{RequiredContentKind, UsedContentKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CONSUMER_PREFIX: &str = "hud_role/v1";
pub const MAX_ROOTS_PER_SNAPSHOT: usize = 64;
pub const MAX_BINDINGS_PER_SURFACE: usize = 65_536;
pub const MAX_RUNTIME_ROOT_NAME_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HudSurface {
    Topbar,
    Minimap,
    Mapmodes,
    TheatreSelector,
    DesignerCountryEquipment,
    DesignerTank,
    DesignerPlane,
}

impl HudSurface {
    pub fn token(self) -> &'static str {
        match self {
            Self::Topbar => "topbar",
            Self::Minimap => "minimap",
            Self::Mapmodes => "mapmodes",
            Self::TheatreSelector => "theatre_selector",
            Self::DesignerCountryEquipment => "designer/country_equipment",
            Self::DesignerTank => "designer/tank",
            Self::DesignerPlane => "designer/plane",
        }
    }
}

pub const ALL_HUD_SURFACES: [HudSurface; 7] = [
    HudSurface::Topbar,
    HudSurface::Minimap,
    HudSurface::Mapmodes,
    HudSurface::TheatreSelector,
    HudSurface::DesignerCountryEquipment,
    HudSurface::DesignerTank,
    HudSurface::DesignerPlane,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudSurfaceActivation {
    pub surface: HudSurface,
    pub runtime_root_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudSurfaceActivationFrame {
    pub frame_hour: u64,
    pub status_flags: u32,
    pub observation_sequence: u64,
    pub active_surfaces: Vec<HudSurfaceActivation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum HudSemanticRole {
    GuiRoot {
        window_or_root: String,
        layout_identity_sha256: String,
    },
    Sprite {
        element: String,
        sprite_id: String,
    },
    SpriteTexture {
        element: String,
        sprite_id: String,
        texture_slot: String,
    },
    SpriteEffect {
        element: String,
        sprite_id: String,
        effect_token: String,
    },
    SpriteEffectInclude {
        element: String,
        sprite_id: String,
        include_slot: String,
    },
    Font {
        element: String,
        font_id: String,
    },
    FontTexture {
        element: String,
        font_id: String,
        texture_slot: String,
    },
    Localisation {
        consumer: String,
        localisation_key: String,
    },
    DefineOrScriptedConsumer {
        consumer: String,
        source_identity: String,
    },
}

impl HudSemanticRole {
    pub fn required_kind(&self) -> RequiredContentKind {
        match self {
            Self::GuiRoot { .. } => RequiredContentKind::GuiDefinition,
            Self::Sprite { .. } => RequiredContentKind::Catalog,
            Self::SpriteTexture { .. } | Self::FontTexture { .. } => RequiredContentKind::Texture,
            Self::SpriteEffect { .. } | Self::SpriteEffectInclude { .. } => {
                RequiredContentKind::ShaderSource
            }
            Self::Font { .. } => RequiredContentKind::Font,
            Self::Localisation { .. } => RequiredContentKind::Localisation,
            Self::DefineOrScriptedConsumer { .. } => RequiredContentKind::Definition,
        }
    }

    pub fn consumer(&self, surface: HudSurface) -> Result<String, String> {
        let suffix = match self {
            Self::GuiRoot {
                window_or_root,
                layout_identity_sha256,
            } => {
                validate_token(window_or_root, "HUD window/root")?;
                validate_sha256(layout_identity_sha256, "HUD layout identity")?;
                format!("gui/{window_or_root}/{layout_identity_sha256}")
            }
            Self::Sprite { element, sprite_id } => {
                validate_token(element, "HUD sprite element")?;
                validate_token(sprite_id, "HUD sprite id")?;
                format!("sprite/{element}/{sprite_id}")
            }
            Self::SpriteTexture {
                element,
                sprite_id,
                texture_slot,
            } => {
                validate_token(element, "HUD sprite-texture element")?;
                validate_token(sprite_id, "HUD sprite-texture id")?;
                validate_token(texture_slot, "HUD sprite-texture slot")?;
                format!("sprite_texture/{element}/{sprite_id}/{texture_slot}")
            }
            Self::SpriteEffect {
                element,
                sprite_id,
                effect_token,
            } => {
                validate_token(element, "HUD sprite-effect element")?;
                validate_token(sprite_id, "HUD sprite-effect id")?;
                validate_token(effect_token, "HUD sprite-effect token")?;
                format!("sprite_effect/{element}/{sprite_id}/{effect_token}")
            }
            Self::SpriteEffectInclude {
                element,
                sprite_id,
                include_slot,
            } => {
                validate_token(element, "HUD sprite-effect-include element")?;
                validate_token(sprite_id, "HUD sprite-effect-include id")?;
                validate_token(include_slot, "HUD sprite-effect-include slot")?;
                format!("sprite_effect_include/{element}/{sprite_id}/{include_slot}")
            }
            Self::Font { element, font_id } => {
                validate_token(element, "HUD font element")?;
                validate_token(font_id, "HUD font id")?;
                format!("font/{element}/{font_id}")
            }
            Self::FontTexture {
                element,
                font_id,
                texture_slot,
            } => {
                validate_token(element, "HUD font-texture element")?;
                validate_token(font_id, "HUD font-texture id")?;
                validate_token(texture_slot, "HUD font-texture slot")?;
                format!("font_texture/{element}/{font_id}/{texture_slot}")
            }
            Self::Localisation {
                consumer,
                localisation_key,
            } => {
                validate_token(consumer, "HUD localisation consumer")?;
                validate_token(localisation_key, "HUD localisation key")?;
                format!("localisation/{consumer}/{localisation_key}")
            }
            Self::DefineOrScriptedConsumer {
                consumer,
                source_identity,
            } => {
                validate_token(consumer, "HUD define/scripted consumer")?;
                validate_token(source_identity, "HUD define/scripted source identity")?;
                format!("define_or_scripted/{consumer}/{source_identity}")
            }
        };
        Ok(format!("{CONSUMER_PREFIX}/{}/{suffix}", surface.token()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedHudRoleBinding {
    pub semantic_role: HudSemanticRole,
    pub content: UsedContentKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedHudSurface {
    pub complete: bool,
    pub bindings: Vec<ResolvedHudRoleBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudRuntimeRootIdentity {
    pub setup_rva: u32,
    pub surface: HudSurface,
    pub runtime_root_name: String,
}

pub trait ExactHudRoleResolver {
    fn authority_manifest_sha256(&self) -> &str;
    fn resolve_surface(&self, root: &HudRuntimeRootIdentity) -> Option<ResolvedHudSurface>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedHudSurfaceClosure {
    pub surface: HudSurface,
    pub runtime_root_name: String,
    pub complete_dependency_closure: bool,
    pub dependency_closure_sha256: String,
    #[serde(default)]
    pub missing_roles: Vec<String>,
    pub bindings: Vec<ResolvedHudRoleBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveVfsHudRoleIndexDocument {
    pub schema: String,
    pub authority_manifest_sha256: String,
    pub source_inventory_sha256: String,
    pub winning_content: Vec<UsedContentKey>,
    pub closures: Vec<AuthenticatedHudSurfaceClosure>,
}

impl EffectiveVfsHudRoleIndexDocument {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != "extractor.effective-vfs-hud-role-index.v1" {
            return Err("HUD role index schema is unsupported".into());
        }
        validate_sha256(&self.authority_manifest_sha256, "HUD role index authority")?;
        if self.source_inventory_sha256 != self.authority_manifest_sha256 {
            return Err("HUD role index is not bound to its source inventory".into());
        }
        let mut winners = BTreeMap::new();
        for content in &self.winning_content {
            content.validate()?;
            if content.effective_vfs_manifest_sha256 != self.authority_manifest_sha256 {
                return Err("HUD winning-content row names a different effective VFS".into());
            }
            if winners
                .insert(content.normalized_game_relative_path.as_str(), content)
                .is_some()
            {
                return Err("HUD winning-content inventory repeats a path".into());
            }
        }
        if winners.is_empty() {
            return Err("HUD winning-content inventory is empty".into());
        }
        let mut closure_keys = BTreeSet::new();
        for closure in &self.closures {
            validate_runtime_root_name(&closure.runtime_root_name)?;
            validate_sha256(&closure.dependency_closure_sha256, "HUD dependency closure")?;
            if !closure_keys.insert((closure.surface, closure.runtime_root_name.as_str())) {
                return Err("HUD role index repeats a runtime surface".into());
            }
            if closure.complete_dependency_closure && !closure.missing_roles.is_empty() {
                return Err("complete HUD dependency closure reports missing roles".into());
            }
            if closure.bindings.len() > MAX_BINDINGS_PER_SURFACE {
                return Err("HUD surface role list exceeds its bound".into());
            }
            let mut consumers = BTreeSet::new();
            let mut gui_roots = 0usize;
            for binding in &closure.bindings {
                binding.content.validate()?;
                let expected = winners
                    .get(binding.content.normalized_game_relative_path.as_str())
                    .ok_or("HUD role is absent from the effective-VFS winning inventory")?;
                if *expected != &binding.content {
                    return Err("HUD role does not equal its effective-VFS winning row".into());
                }
                if matches!(binding.semantic_role, HudSemanticRole::GuiRoot { .. }) {
                    gui_roots += 1;
                }
                if !consumers.insert(binding.semantic_role.consumer(closure.surface)?) {
                    return Err("HUD dependency closure repeats a semantic consumer".into());
                }
            }
            if closure.complete_dependency_closure
                && (closure.bindings.is_empty() || gui_roots != 1)
            {
                return Err("complete HUD dependency closure lacks exactly one GUI root".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudContentObservation {
    pub content_key: String,
    pub kind: RequiredContentKind,
    pub content: UsedContentKey,
    pub consumer: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OmittedHudSurface {
    pub surface: HudSurface,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudRoleSnapshot {
    pub schema: String,
    pub authority_manifest_sha256: String,
    pub activated_surfaces: Vec<HudSurface>,
    pub observations: Vec<HudContentObservation>,
    pub omitted_surfaces: Vec<OmittedHudSurface>,
}

impl HudRoleSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != "extractor.hud-role-snapshot.v1" {
            return Err("HUD role snapshot schema is unsupported".into());
        }
        validate_sha256(&self.authority_manifest_sha256, "HUD snapshot authority")?;
        if self.activated_surfaces.is_empty()
            || self.activated_surfaces.len() > MAX_ROOTS_PER_SNAPSHOT
        {
            return Err(
                "HUD snapshot activated-surface partition is empty or above its bound".into(),
            );
        }
        let mut activated = BTreeSet::new();
        let mut prior = None;
        for surface in &self.activated_surfaces {
            if !activated.insert(*surface) || prior.is_some_and(|old| old >= *surface) {
                return Err("HUD snapshot activated surfaces are repeated or non-canonical".into());
            }
            prior = Some(*surface);
        }
        let mut omitted = BTreeSet::new();
        let mut prior_omitted = None;
        for row in &self.omitted_surfaces {
            if row.reason.trim().is_empty()
                || !activated.contains(&row.surface)
                || !omitted.insert(row.surface)
                || prior_omitted.is_some_and(|old| old >= row.surface)
            {
                return Err(
                    "HUD snapshot omissions are blank, repeated, non-canonical, or inactive".into(),
                );
            }
            prior_omitted = Some(row.surface);
        }
        let mut observed = BTreeSet::new();
        let mut gui_counts = BTreeMap::<HudSurface, usize>::new();
        let mut prior_observation: Option<(&str, &str)> = None;
        for row in &self.observations {
            row.content.validate()?;
            if row.content_key != row.content.normalized_game_relative_path
                || row.content.effective_vfs_manifest_sha256 != self.authority_manifest_sha256
            {
                return Err("HUD snapshot observation is not bound to its authority/path".into());
            }
            if prior_observation
                .is_some_and(|prior| prior >= (row.content_key.as_str(), row.consumer.as_str()))
            {
                return Err("HUD snapshot observations are repeated or non-canonical".into());
            }
            prior_observation = Some((&row.content_key, &row.consumer));
            let surface = surface_from_consumer(&row.consumer)?;
            if !activated.contains(&surface) || omitted.contains(&surface) {
                return Err(
                    "HUD snapshot observed and omitted surfaces overlap or are inactive".into(),
                );
            }
            observed.insert(surface);
            if row
                .consumer
                .starts_with(&format!("{CONSUMER_PREFIX}/{}/gui/", surface.token()))
            {
                *gui_counts.entry(surface).or_default() += 1;
            }
        }
        for surface in &observed {
            if gui_counts.get(surface).copied() != Some(1) {
                return Err("HUD snapshot observed surface lacks exactly one GUI root".into());
            }
        }
        if activated != observed.union(&omitted).copied().collect() {
            return Err("HUD snapshot does not completely partition activated surfaces".into());
        }
        Ok(())
    }
}

pub fn validate_token(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(format!("{label} is blank or not a neutral token"));
    }
    Ok(())
}

pub fn validate_runtime_root_name(value: &str) -> Result<(), String> {
    if value.len() > MAX_RUNTIME_ROOT_NAME_BYTES {
        return Err("HUD runtime root name exceeds its bound".into());
    }
    validate_token(value, "HUD runtime root name")
}

pub fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{label} is not lowercase SHA-256"));
    }
    Ok(())
}

fn surface_from_consumer(consumer: &str) -> Result<HudSurface, String> {
    ALL_HUD_SURFACES
        .into_iter()
        .find(|surface| consumer.starts_with(&format!("{CONSUMER_PREFIX}/{}/", surface.token())))
        .ok_or_else(|| "HUD observation consumer has no authenticated surface prefix".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_and_root_name_validation_fail_closed() {
        assert!(validate_token("topbar_root", "token").is_ok());
        assert!(validate_token("", "token").is_err());
        assert!(validate_token("   ", "token").is_err());
        assert!(validate_token("a/b", "token").is_err());
        assert!(validate_token("a\\b", "token").is_err());
        assert!(validate_token("a\u{7}b", "token").is_err());

        assert!(validate_runtime_root_name("hud_root").is_ok());
        let oversized = "r".repeat(MAX_RUNTIME_ROOT_NAME_BYTES + 1);
        assert_eq!(
            validate_runtime_root_name(&oversized).unwrap_err(),
            "HUD runtime root name exceeds its bound"
        );
    }

    #[test]
    fn sha256_validation_accepts_only_64_lowercase_hex() {
        assert!(validate_sha256(&"a".repeat(64), "digest").is_ok());
        assert_eq!(
            validate_sha256(&"A".repeat(64), "digest").unwrap_err(),
            "digest is not lowercase SHA-256"
        );
        assert!(validate_sha256(&"a".repeat(63), "digest").is_err());
        assert!(validate_sha256(&"g".repeat(64), "digest").is_err());
        assert!(validate_sha256("", "digest").is_err());
    }

    #[test]
    fn semantic_role_consumers_are_surface_scoped_and_validated() {
        let role = HudSemanticRole::Sprite {
            element: "topbar_e0_icon".into(),
            sprite_id: "GFX_topbar".into(),
        };
        assert_eq!(
            role.consumer(HudSurface::Topbar).unwrap(),
            "hud_role/v1/topbar/sprite/topbar_e0_icon/GFX_topbar"
        );
        let bad = HudSemanticRole::Sprite {
            element: "with/slash".into(),
            sprite_id: "GFX_topbar".into(),
        };
        assert!(bad.consumer(HudSurface::Topbar).is_err());
        let root = HudSemanticRole::GuiRoot {
            window_or_root: "hud_root".into(),
            layout_identity_sha256: "b".repeat(64),
        };
        assert_eq!(
            root.consumer(HudSurface::Minimap).unwrap(),
            format!("hud_role/v1/minimap/gui/hud_root/{}", "b".repeat(64))
        );
        let effect = HudSemanticRole::SpriteEffect {
            element: "topbar_e0_icon".into(),
            sprite_id: "GFX_topbar".into(),
            effect_token: "gfx_FX_buttonstate_shader".into(),
        };
        assert_eq!(effect.required_kind(), RequiredContentKind::ShaderSource);
        assert_eq!(
            effect.consumer(HudSurface::Topbar).unwrap(),
            "hud_role/v1/topbar/sprite_effect/topbar_e0_icon/GFX_topbar/gfx_FX_buttonstate_shader"
        );
    }
}
