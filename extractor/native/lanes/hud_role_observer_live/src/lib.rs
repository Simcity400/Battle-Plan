//! Authenticated, VFS-neutral HUD semantic-role observation for a729.
//!
//! This lane never serializes process pointers or assumes vanilla paths.  The
//! hook observes only proven controller roots.  A resolver owned by the live
//! effective-VFS interpreter must then bind the runtime root to the winning GUI
//! source and all roles actually consumed beneath it.  An incomplete surface is
//! omitted as a unit; it is never completed from a frozen vanilla catalog.

use portable_content_manifest::hud_role::{validate_runtime_root_name, validate_sha256};
pub use portable_content_manifest::hud_role::{
    AuthenticatedHudSurfaceClosure, EffectiveVfsHudRoleIndexDocument, ExactHudRoleResolver,
    HudContentObservation, HudRoleSnapshot, HudRuntimeRootIdentity, HudSemanticRole, HudSurface,
    HudSurfaceActivation, HudSurfaceActivationFrame, OmittedHudSurface, ResolvedHudRoleBinding,
    ResolvedHudSurface, ALL_HUD_SURFACES, CONSUMER_PREFIX, MAX_BINDINGS_PER_SURFACE,
    MAX_ROOTS_PER_SNAPSHOT, MAX_RUNTIME_ROOT_NAME_BYTES,
};
use portable_content_manifest::UsedContentKey;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(windows)]
pub mod a729_hook;

pub const DOMAIN_ID: u16 = 79;
pub const DOMAIN_NAME: &str = "hud_surface_activation_live";
pub const ACTIVATION_MAGIC: u32 = u32::from_le_bytes(*b"HRA1");
pub const ACTIVATION_SCHEMA_VERSION: u32 = 1;

pub const TOPBAR_SETUP_RVA: u32 = 0x0188_BA30;
pub const MINIMAP_SETUP_RVA: u32 = 0x017B_9A80;
pub const MAPMODE_SETUP_RVA: u32 = 0x0156_3C80;
pub const THEATRE_SETUP_RVA: u32 = 0x0187_8E40;
pub const DESIGNER_REFRESH_RVA: u32 = 0x0177_97A0;

/// Pointer-bearing observation which is valid only inside the recording
/// process.  These values must never cross the portable replay boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawHudControllerRoot {
    pub sequence: u64,
    pub setup_rva: u32,
    pub surface: HudSurface,
    pub controller_ptr: usize,
    pub runtime_root_ptr: usize,
    /// Exact authored object identity returned by the runtime GUI definition's
    /// proven virtual getter. This is captured after setup and is the only key
    /// the effective-VFS interpreter may use to select an authored GUI root.
    pub runtime_root_name: String,
}

pub fn encode_activation_payload(frame: &HudSurfaceActivationFrame) -> Result<Vec<u8>, String> {
    validate_activation_frame(frame)?;
    let mut out = Vec::with_capacity(32 + frame.active_surfaces.len() * 24);
    out.extend_from_slice(&ACTIVATION_MAGIC.to_le_bytes());
    out.extend_from_slice(&ACTIVATION_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&frame.status_flags.to_le_bytes());
    out.extend_from_slice(&frame.frame_hour.to_le_bytes());
    out.extend_from_slice(&frame.observation_sequence.to_le_bytes());
    out.extend_from_slice(&(frame.active_surfaces.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    for row in &frame.active_surfaces {
        out.push(surface_code(row.surface));
        out.push(0);
        out.extend_from_slice(&(row.runtime_root_name.len() as u16).to_le_bytes());
        out.extend_from_slice(row.runtime_root_name.as_bytes());
    }
    Ok(out)
}

pub fn decode_activation_payload(payload: &[u8]) -> Result<HudSurfaceActivationFrame, String> {
    if payload.len() < 32 {
        return Err("HUD activation payload has an invalid length".into());
    }
    let u32_at = |at| u32::from_le_bytes(payload[at..at + 4].try_into().unwrap());
    let u64_at = |at| u64::from_le_bytes(payload[at..at + 8].try_into().unwrap());
    if u32_at(0) != ACTIVATION_MAGIC || u32_at(4) != ACTIVATION_SCHEMA_VERSION {
        return Err("HUD activation payload has an invalid magic/schema".into());
    }
    let count = u16::from_le_bytes(payload[28..30].try_into().unwrap()) as usize;
    if payload[30..32] != [0, 0] || count > ALL_HUD_SURFACES.len() {
        return Err("HUD activation payload has invalid count/reserved fields".into());
    }
    let mut active_surfaces = Vec::with_capacity(count);
    let mut cursor = 32usize;
    for _ in 0..count {
        if cursor.saturating_add(4) > payload.len() || payload[cursor + 1] != 0 {
            return Err("HUD activation row header is truncated or invalid".into());
        }
        let surface = surface_from_code(payload[cursor])?;
        let name_len =
            u16::from_le_bytes(payload[cursor + 2..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        let end = cursor
            .checked_add(name_len)
            .filter(|end| *end <= payload.len())
            .ok_or("HUD activation root name is truncated")?;
        let runtime_root_name = std::str::from_utf8(&payload[cursor..end])
            .map_err(|_| "HUD activation root name is not UTF-8")?
            .to_owned();
        active_surfaces.push(HudSurfaceActivation {
            surface,
            runtime_root_name,
        });
        cursor = end;
    }
    if cursor != payload.len() {
        return Err("HUD activation payload has trailing bytes".into());
    }
    let frame = HudSurfaceActivationFrame {
        frame_hour: u64_at(12),
        status_flags: u32_at(8),
        observation_sequence: u64_at(20),
        active_surfaces,
    };
    validate_activation_frame(&frame)?;
    Ok(frame)
}

fn surface_code(surface: HudSurface) -> u8 {
    match surface {
        HudSurface::Topbar => 0,
        HudSurface::Minimap => 1,
        HudSurface::Mapmodes => 2,
        HudSurface::TheatreSelector => 3,
        HudSurface::DesignerCountryEquipment => 4,
        HudSurface::DesignerTank => 5,
        HudSurface::DesignerPlane => 6,
    }
}

fn surface_from_code(code: u8) -> Result<HudSurface, String> {
    ALL_HUD_SURFACES
        .get(code as usize)
        .copied()
        .ok_or_else(|| "HUD activation payload contains an unknown surface code".into())
}

fn validate_activations(surfaces: &[HudSurfaceActivation]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let mut designer_count = 0;
    let mut previous = None;
    for row in surfaces {
        if !seen.insert(row.surface) {
            return Err("HUD activation repeats a surface".into());
        }
        if previous.is_some_and(|prior| prior >= row.surface) {
            return Err("HUD activation surfaces are not in canonical order".into());
        }
        previous = Some(row.surface);
        validate_runtime_root_name(&row.runtime_root_name)?;
        if matches!(
            row.surface,
            HudSurface::DesignerCountryEquipment
                | HudSurface::DesignerTank
                | HudSurface::DesignerPlane
        ) {
            designer_count += 1;
        }
    }
    if designer_count > 1 {
        return Err("HUD activation contains multiple active designer families".into());
    }
    Ok(())
}

fn validate_activation_frame(frame: &HudSurfaceActivationFrame) -> Result<(), String> {
    if frame.observation_sequence == 0 {
        return Err("HUD activation frame has zero sequence".into());
    }
    if frame.active_surfaces.is_empty() && frame.status_flags == 0 {
        return Err("successful HUD activation frame is empty".into());
    }
    validate_activations(&frame.active_surfaces)?;
    Ok(())
}

/// Concrete live resolver.  Construction cross-checks every interpreted role
/// against the exact winning row from the finalized source inventory.  This
/// prevents an otherwise well-formed authored sidecar from naming a stale,
/// shadowed, or guessed vanilla path.
#[derive(Clone, Debug)]
pub struct EffectiveVfsHudRoleResolver {
    authority_manifest_sha256: String,
    closures: BTreeMap<(HudSurface, String), ResolvedHudSurface>,
}

impl EffectiveVfsHudRoleResolver {
    pub fn decode_authenticated_index(bytes: &[u8]) -> Result<Self, String> {
        let document: EffectiveVfsHudRoleIndexDocument = serde_json::from_slice(bytes)
            .map_err(|error| format!("parse HUD role index failed: {error}"))?;
        document.validate()?;
        Self::new(
            document.authority_manifest_sha256,
            document.winning_content,
            document.closures,
        )
    }

    pub fn new(
        authority_manifest_sha256: String,
        winning_content: impl IntoIterator<Item = UsedContentKey>,
        closures: impl IntoIterator<Item = AuthenticatedHudSurfaceClosure>,
    ) -> Result<Self, String> {
        validate_sha256(
            &authority_manifest_sha256,
            "HUD resolver authority manifest",
        )?;
        let mut winners = BTreeMap::new();
        for content in winning_content {
            content.validate()?;
            if content.effective_vfs_manifest_sha256 != authority_manifest_sha256 {
                return Err("HUD winning-content row names a different effective VFS".into());
            }
            let path = content.normalized_game_relative_path.clone();
            if winners.insert(path, content).is_some() {
                return Err("HUD winning-content inventory repeats a path".into());
            }
        }
        if winners.is_empty() {
            return Err("HUD winning-content inventory is empty".into());
        }

        let mut resolved = BTreeMap::new();
        for closure in closures {
            validate_runtime_root_name(&closure.runtime_root_name)?;
            validate_sha256(&closure.dependency_closure_sha256, "HUD dependency closure")?;
            if closure.complete_dependency_closure && !closure.missing_roles.is_empty() {
                return Err("complete HUD dependency closure reports missing roles".into());
            }
            if !closure.complete_dependency_closure || closure.bindings.is_empty() {
                continue;
            }
            if closure.bindings.len() > MAX_BINDINGS_PER_SURFACE {
                return Err("HUD surface role list exceeds its bound".into());
            }
            let mut consumers = BTreeSet::new();
            let mut gui_roots = 0usize;
            for binding in &closure.bindings {
                binding.content.validate()?;
                let expected = winners
                    .get(&binding.content.normalized_game_relative_path)
                    .ok_or("HUD role is absent from the effective-VFS winning inventory")?;
                if expected != &binding.content {
                    return Err("HUD role does not equal its effective-VFS winning row".into());
                }
                if matches!(binding.semantic_role, HudSemanticRole::GuiRoot { .. }) {
                    gui_roots += 1;
                }
                let consumer = binding.semantic_role.consumer(closure.surface)?;
                if !consumers.insert(consumer) {
                    return Err("HUD dependency closure repeats a semantic consumer".into());
                }
            }
            if gui_roots != 1 {
                return Err("HUD dependency closure must have exactly one GUI root".into());
            }
            let surface = closure.surface;
            let runtime_root_name = closure.runtime_root_name;
            if resolved
                .insert(
                    (surface, runtime_root_name),
                    ResolvedHudSurface {
                        complete: true,
                        bindings: closure.bindings,
                    },
                )
                .is_some()
            {
                return Err("HUD resolver repeats a runtime surface".into());
            }
        }
        Ok(Self {
            authority_manifest_sha256,
            closures: resolved,
        })
    }

    pub fn promoted_surfaces(&self) -> impl Iterator<Item = HudSurface> + '_ {
        self.closures.keys().map(|(surface, _)| *surface)
    }
}

impl ExactHudRoleResolver for EffectiveVfsHudRoleResolver {
    fn authority_manifest_sha256(&self) -> &str {
        &self.authority_manifest_sha256
    }

    fn resolve_surface(&self, root: &HudRuntimeRootIdentity) -> Option<ResolvedHudSurface> {
        (root.setup_rva == setup_rva(root.surface))
            .then(|| {
                self.closures
                    .get(&(root.surface, root.runtime_root_name.clone()))
                    .cloned()
            })
            .flatten()
    }
}

pub fn promote_snapshot<R: ExactHudRoleResolver>(
    roots: &[RawHudControllerRoot],
    resolver: &R,
) -> Result<HudRoleSnapshot, String> {
    validate_sha256(
        resolver.authority_manifest_sha256(),
        "HUD resolver authority manifest",
    )?;
    if roots.is_empty() || roots.len() > MAX_ROOTS_PER_SNAPSHOT {
        return Err("HUD root snapshot is empty or above its bound".into());
    }
    let mut prior_sequence = 0;
    let mut seen_surfaces = BTreeSet::new();
    let mut identities = Vec::with_capacity(roots.len());
    for root in roots {
        if root.sequence == 0
            || root.sequence <= prior_sequence
            || root.controller_ptr == 0
            || root.runtime_root_ptr == 0
            || root.setup_rva != setup_rva(root.surface)
            || !seen_surfaces.insert(root.surface)
        {
            return Err("HUD controller roots are invalid, duplicated, or out of order".into());
        }
        prior_sequence = root.sequence;
        validate_runtime_root_name(&root.runtime_root_name)?;
        identities.push(HudRuntimeRootIdentity {
            setup_rva: root.setup_rva,
            surface: root.surface,
            runtime_root_name: root.runtime_root_name.clone(),
        });
    }
    promote_identities(&identities, resolver)
}

/// Promotes one latest-clean d79 frame without reconstructing or inventing any
/// process pointer. Failed/dirty d79 frames are not eligible for promotion.
pub fn promote_activation_frame<R: ExactHudRoleResolver>(
    frame: &HudSurfaceActivationFrame,
    resolver: &R,
) -> Result<HudRoleSnapshot, String> {
    validate_activation_frame(frame)?;
    if frame.status_flags != 0 {
        return Err("failed or badged HUD activation frame is not promotion-eligible".into());
    }
    let identities = frame
        .active_surfaces
        .iter()
        .map(|row| HudRuntimeRootIdentity {
            setup_rva: setup_rva(row.surface),
            surface: row.surface,
            runtime_root_name: row.runtime_root_name.clone(),
        })
        .collect::<Vec<_>>();
    promote_identities(&identities, resolver)
}

fn promote_identities<R: ExactHudRoleResolver>(
    roots: &[HudRuntimeRootIdentity],
    resolver: &R,
) -> Result<HudRoleSnapshot, String> {
    validate_sha256(
        resolver.authority_manifest_sha256(),
        "HUD resolver authority manifest",
    )?;
    if roots.is_empty() || roots.len() > MAX_ROOTS_PER_SNAPSHOT {
        return Err("HUD root identity snapshot is empty or above its bound".into());
    }
    let mut seen_surfaces = BTreeSet::new();
    let mut observations = Vec::new();
    let mut omitted_surfaces = Vec::new();
    for root in roots {
        if root.setup_rva != setup_rva(root.surface) || !seen_surfaces.insert(root.surface) {
            return Err("HUD root identities are invalid or duplicated".into());
        }
        validate_runtime_root_name(&root.runtime_root_name)?;
        let Some(resolved) = resolver.resolve_surface(root) else {
            omitted_surfaces.push(OmittedHudSurface {
                surface: root.surface,
                reason: "runtime root did not resolve through the live effective VFS".into(),
            });
            continue;
        };
        if !resolved.complete || resolved.bindings.is_empty() {
            omitted_surfaces.push(OmittedHudSurface {
                surface: root.surface,
                reason: "surface has an unknown or incomplete authored semantic role".into(),
            });
            continue;
        }
        if resolved.bindings.len() > MAX_BINDINGS_PER_SURFACE {
            return Err("HUD surface role list exceeds its bound".into());
        }
        let mut surface_rows = Vec::with_capacity(resolved.bindings.len());
        let mut has_gui_root = false;
        let mut seen_consumers = BTreeSet::new();
        for binding in resolved.bindings {
            binding.content.validate()?;
            if binding.content.effective_vfs_manifest_sha256 != resolver.authority_manifest_sha256()
            {
                return Err("HUD role names a different effective VFS".into());
            }
            has_gui_root |= matches!(binding.semantic_role, HudSemanticRole::GuiRoot { .. });
            let kind = binding.semantic_role.required_kind();
            let consumer = binding.semantic_role.consumer(root.surface)?;
            if !seen_consumers.insert(consumer.clone()) {
                return Err("HUD surface repeats a semantic consumer".into());
            }
            surface_rows.push(HudContentObservation {
                content_key: binding.content.normalized_game_relative_path.clone(),
                kind,
                content: binding.content,
                consumer,
            });
        }
        if !has_gui_root {
            omitted_surfaces.push(OmittedHudSurface {
                surface: root.surface,
                reason: "surface has no authenticated GUI root binding".into(),
            });
            continue;
        }
        observations.extend(surface_rows);
    }
    observations.sort_by(|left, right| {
        (&left.content_key, &left.consumer).cmp(&(&right.content_key, &right.consumer))
    });
    omitted_surfaces.sort_by_key(|row| row.surface);
    let mut activated_surfaces = roots.iter().map(|root| root.surface).collect::<Vec<_>>();
    activated_surfaces.sort();
    let snapshot = HudRoleSnapshot {
        schema: "extractor.hud-role-snapshot.v1".into(),
        authority_manifest_sha256: resolver.authority_manifest_sha256().into(),
        activated_surfaces,
        observations,
        omitted_surfaces,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

fn setup_rva(surface: HudSurface) -> u32 {
    match surface {
        HudSurface::Topbar => TOPBAR_SETUP_RVA,
        HudSurface::Minimap => MINIMAP_SETUP_RVA,
        HudSurface::Mapmodes => MAPMODE_SETUP_RVA,
        HudSurface::TheatreSelector => THEATRE_SETUP_RVA,
        HudSurface::DesignerCountryEquipment
        | HudSurface::DesignerTank
        | HudSurface::DesignerPlane => DESIGNER_REFRESH_RVA,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn h(ch: char) -> String {
        ch.to_string().repeat(64)
    }

    fn key(path: &str) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: h('a'),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 3,
            layer_kind: ContentLayerKind::Mod,
            layer_id: "mod/override.mod".into(),
            source_sha256: h('b'),
            payload_sha256: h('c'),
            payload_bytes: 17,
        }
    }

    struct Resolver {
        complete: bool,
    }

    impl ExactHudRoleResolver for Resolver {
        fn authority_manifest_sha256(&self) -> &str {
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }

        fn resolve_surface(&self, _: &HudRuntimeRootIdentity) -> Option<ResolvedHudSurface> {
            Some(ResolvedHudSurface {
                complete: self.complete,
                bindings: vec![
                    ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::GuiRoot {
                            window_or_root: "topbar".into(),
                            layout_identity_sha256: h('d'),
                        },
                        content: key("interface/modded_topbar.gui"),
                    },
                    ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::Sprite {
                            element: "player_flag".into(),
                            sprite_id: "GFX_modded_shield".into(),
                        },
                        content: key("interface/modded_topbar.gfx"),
                    },
                ],
            })
        }
    }

    fn root() -> RawHudControllerRoot {
        RawHudControllerRoot {
            sequence: 1,
            setup_rva: TOPBAR_SETUP_RVA,
            surface: HudSurface::Topbar,
            controller_ptr: 0x1000,
            runtime_root_ptr: 0x2000,
            runtime_root_name: "mod_topbar".into(),
        }
    }

    #[test]
    fn mod_override_roles_are_portable_and_no_vanilla_path_is_assumed() {
        let snapshot = promote_snapshot(&[root()], &Resolver { complete: true }).unwrap();
        assert_eq!(snapshot.observations.len(), 2);
        assert!(snapshot.omitted_surfaces.is_empty());
        assert!(snapshot
            .observations
            .iter()
            .all(|row| row.consumer.starts_with("hud_role/v1/topbar/")));
        assert!(snapshot
            .observations
            .iter()
            .all(|row| row.content.layer_id == "mod/override.mod"));
    }

    #[test]
    fn incomplete_surface_is_omitted_as_a_unit() {
        let snapshot = promote_snapshot(&[root()], &Resolver { complete: false }).unwrap();
        assert!(snapshot.observations.is_empty());
        assert_eq!(snapshot.omitted_surfaces.len(), 1);
        snapshot.validate().unwrap();
    }

    #[test]
    fn snapshot_validation_rejects_partition_bypasses() {
        let valid = promote_snapshot(&[root()], &Resolver { complete: true }).unwrap();
        valid.validate().unwrap();

        let mut overlap = valid.clone();
        overlap.omitted_surfaces.push(OmittedHudSurface {
            surface: HudSurface::Topbar,
            reason: "fabricated omission".into(),
        });
        assert!(overlap.validate().is_err());

        let mut incomplete = valid.clone();
        incomplete.activated_surfaces.push(HudSurface::Minimap);
        assert!(incomplete.validate().is_err());

        let mut duplicate = promote_snapshot(&[root()], &Resolver { complete: false }).unwrap();
        duplicate
            .omitted_surfaces
            .push(duplicate.omitted_surfaces[0].clone());
        assert!(duplicate.validate().is_err());

        let mut value = serde_json::to_value(&valid).unwrap();
        value.as_object_mut().unwrap().remove("activated_surfaces");
        assert!(serde_json::from_value::<HudRoleSnapshot>(value).is_err());
    }

    #[test]
    fn wrong_hook_root_and_foreign_vfs_fail_closed() {
        let mut wrong = root();
        wrong.setup_rva = MINIMAP_SETUP_RVA;
        assert!(promote_snapshot(&[wrong], &Resolver { complete: true }).is_err());
        struct Foreign;
        impl ExactHudRoleResolver for Foreign {
            fn authority_manifest_sha256(&self) -> &str {
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            }
            fn resolve_surface(&self, root: &HudRuntimeRootIdentity) -> Option<ResolvedHudSurface> {
                Resolver { complete: true }.resolve_surface(root)
            }
        }
        assert!(promote_snapshot(&[root()], &Foreign).is_err());
    }

    #[test]
    fn concrete_resolver_accepts_only_exact_mod_winner_and_complete_closure() {
        let gui = key("interface/modded_topbar.gui");
        let gfx = key("interface/modded_topbar.gfx");
        let resolver = EffectiveVfsHudRoleResolver::new(
            h('a'),
            [gui.clone(), gfx.clone()],
            [AuthenticatedHudSurfaceClosure {
                surface: HudSurface::Topbar,
                runtime_root_name: "mod_topbar".into(),
                complete_dependency_closure: true,
                dependency_closure_sha256: h('d'),
                missing_roles: vec![],
                bindings: vec![
                    ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::GuiRoot {
                            window_or_root: "mod_topbar".into(),
                            layout_identity_sha256: h('e'),
                        },
                        content: gui,
                    },
                    ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::Sprite {
                            element: "flag".into(),
                            sprite_id: "GFX_mod_flag".into(),
                        },
                        content: gfx,
                    },
                ],
            }],
        )
        .unwrap();
        let snapshot = promote_snapshot(&[root()], &resolver).unwrap();
        assert_eq!(
            resolver.promoted_surfaces().collect::<Vec<_>>(),
            vec![HudSurface::Topbar]
        );
        assert_eq!(snapshot.observations.len(), 2);
        assert!(snapshot
            .observations
            .iter()
            .all(|row| row.content.winning_layer_ordinal == 3));
    }

    #[test]
    fn concrete_resolver_rejects_shadowed_or_unauthenticated_content() {
        let winner = key("interface/overridden.gui");
        let mut shadowed = winner.clone();
        shadowed.winning_layer_ordinal = 0;
        shadowed.layer_id = "base".into();
        assert!(EffectiveVfsHudRoleResolver::new(
            h('a'),
            [winner],
            [AuthenticatedHudSurfaceClosure {
                surface: HudSurface::Topbar,
                runtime_root_name: "mod_topbar".into(),
                complete_dependency_closure: true,
                dependency_closure_sha256: h('d'),
                missing_roles: vec![],
                bindings: vec![ResolvedHudRoleBinding {
                    semantic_role: HudSemanticRole::GuiRoot {
                        window_or_root: "mod_topbar".into(),
                        layout_identity_sha256: h('e'),
                    },
                    content: shadowed,
                }],
            }],
        )
        .is_err());
    }

    #[test]
    fn designer_family_is_a_runtime_role_not_a_path_inference() {
        let gui = key("interface/any_winning_name.gui");
        let resolver = EffectiveVfsHudRoleResolver::new(
            h('a'),
            [gui.clone()],
            [AuthenticatedHudSurfaceClosure {
                surface: HudSurface::DesignerTank,
                runtime_root_name: "runtime_observed_root".into(),
                complete_dependency_closure: true,
                dependency_closure_sha256: h('d'),
                missing_roles: vec![],
                bindings: vec![ResolvedHudRoleBinding {
                    semantic_role: HudSemanticRole::GuiRoot {
                        window_or_root: "runtime_observed_root".into(),
                        layout_identity_sha256: h('e'),
                    },
                    content: gui,
                }],
            }],
        )
        .unwrap();
        let designer = RawHudControllerRoot {
            sequence: 9,
            setup_rva: DESIGNER_REFRESH_RVA,
            surface: HudSurface::DesignerTank,
            controller_ptr: 0x3000,
            runtime_root_ptr: 0x4000,
            runtime_root_name: "runtime_observed_root".into(),
        };
        let snapshot = promote_snapshot(&[designer], &resolver).unwrap();
        assert_eq!(snapshot.observations.len(), 1);
        assert!(snapshot.observations[0]
            .consumer
            .starts_with("hud_role/v1/designer/tank/gui/"));
        assert_eq!(
            snapshot.observations[0].content_key,
            "interface/any_winning_name.gui"
        );
    }

    #[test]
    fn pointer_free_activation_carrier_roundtrips_fixed_and_one_designer_surface() {
        let frame = HudSurfaceActivationFrame {
            frame_hour: 42,
            status_flags: 0,
            observation_sequence: 9,
            active_surfaces: vec![
                activation(HudSurface::Topbar, "mod_topbar"),
                activation(HudSurface::Minimap, "mod_minimap"),
                activation(HudSurface::Mapmodes, "mod_mapmodes"),
                activation(HudSurface::TheatreSelector, "mod_theatre"),
                activation(HudSurface::DesignerPlane, "mod_designer"),
            ],
        };
        let bytes = encode_activation_payload(&frame).unwrap();
        assert!(bytes.len() > 36);
        assert_eq!(decode_activation_payload(&bytes).unwrap(), frame);
        assert!(!bytes
            .windows(std::mem::size_of::<usize>())
            .any(|row| { row == 0x0000_0001_4000_0000usize.to_le_bytes().as_slice() }));
    }

    #[test]
    fn latest_clean_activation_promotes_without_reconstructed_process_pointers() {
        let frame = HudSurfaceActivationFrame {
            frame_hour: 42,
            status_flags: 0,
            observation_sequence: 9,
            active_surfaces: vec![activation(HudSurface::Topbar, "mod_topbar")],
        };
        let snapshot = promote_activation_frame(&frame, &Resolver { complete: true }).unwrap();
        assert_eq!(snapshot.activated_surfaces, vec![HudSurface::Topbar]);
        assert_eq!(snapshot.observations.len(), 2);
        snapshot.validate().unwrap();

        let mut badged = frame;
        badged.status_flags = 1;
        assert!(promote_activation_frame(&badged, &Resolver { complete: true }).is_err());
    }

    #[test]
    fn activation_carrier_rejects_multiple_designer_families_and_unknown_bits() {
        let bad = HudSurfaceActivationFrame {
            frame_hour: 1,
            status_flags: 0,
            observation_sequence: 1,
            active_surfaces: vec![
                activation(HudSurface::DesignerTank, "tank"),
                activation(HudSurface::DesignerPlane, "plane"),
            ],
        };
        assert!(encode_activation_payload(&bad).is_err());
        let good = HudSurfaceActivationFrame {
            frame_hour: 1,
            status_flags: 0,
            observation_sequence: 1,
            active_surfaces: vec![activation(HudSurface::Topbar, "topbar")],
        };
        let mut bytes = encode_activation_payload(&good).unwrap();
        bytes[32] = 0xff;
        assert!(decode_activation_payload(&bytes).is_err());
    }

    fn activation(surface: HudSurface, runtime_root_name: &str) -> HudSurfaceActivation {
        HudSurfaceActivation {
            surface,
            runtime_root_name: runtime_root_name.into(),
        }
    }
}
