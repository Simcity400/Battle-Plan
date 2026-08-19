
use crate::authored_role_index::{
    AuthenticatedGuiRoleIndex, AuthenticatedPanelSurface, AuthoredElementRole,
    StrictAuthoredPanelAdmission,
};
use crate::compose::{authenticate_persistent_hud_surfaces, AuthenticatedPersistentHudPanels};
use crate::definition_admission::GuiDefinitionCatalog;
use crate::hud::HudBinding;
use crate::widget_visual_state::WidgetFamily;
use crate::window_manager_law::{
    AuthenticatedReplayFrameAuthority, AuthenticatedReplayPanelLease, MapmodeViewerAction,
    ReplayAuthenticatedElementSemantic, ReplayElementPointerAuthority, ReplayOwnedWindowManager,
    ReplayViewerAction, ReplayViewerActionBinding, TransportViewerAction,
};
use replay_core::{UsedContentKey, ValidatedContentAdmission};
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;
use std::fmt;

const EXACT_A729_BINARY_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const CONTROLLER_EVIDENCE_BUNDLE_SHA256: &str =
    "e1326692031b6a23bf5e781b8bba5891cc824da4ca241c34a0565c2e9afd3e10";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AuthenticatedHudSemanticSlot {
    TopbarPlayerFlag,
    TopbarPoliticalPower,
    TopbarManpower,
    TopbarStability,
    TopbarWarSupport,
    TopbarCommandPower,
    TopbarArmyExperience,
    TopbarNavyExperience,
    TopbarAirExperience,
    TopbarNukes,
    TopbarIndustrialCapacity,
    TopbarIndustrialRatio,
    TopbarConvoys,
    TopbarConvoysRatio,
    TopbarFuel,
    TopbarFuelRatio,
    TopbarSupply,
    TopbarSupplyRatio,
    TopbarThreatValue,
    TopbarDateText,
    TopbarInitiative,
    TopbarSpeedStep(u8),
    MapmodeThreeButtonLayout,
    MapmodeFourButtonLayout,
    MapmodeButtonDeselected(u8),
    MapmodeButtonSelected(u8),
    TheatreElementsList,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControllerPanel {
    Topbar,
    Mapmodes,
    TheatreRoot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExactA729ControllerClaim {
    slot: AuthenticatedHudSemanticSlot,
    panel: ControllerPanel,
    lookup_token: &'static str,
    controller_rva: u32,
    evidence_job_id: &'static str,
    evidence_artifact_sha256: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedHudSemanticSlotBinding {
    slot: AuthenticatedHudSemanticSlot,
    index_identity_sha256: String,
    source_content: UsedContentKey,
    surface_id: String,
    authored_tree_identity_sha256: String,
    child_path: Vec<u32>,
    engine_lookup_token: String,
    controller_rva: u32,
    evidence_job_id: String,
    evidence_artifact_sha256: String,
}

impl AuthenticatedHudSemanticSlotBinding {
    pub(crate) fn slot(&self) -> AuthenticatedHudSemanticSlot {
        self.slot
    }

    pub(crate) fn source_content(&self) -> &UsedContentKey {
        &self.source_content
    }

    pub(crate) fn surface_id(&self) -> &str {
        &self.surface_id
    }

    pub(crate) fn child_path(&self) -> &[u32] {
        &self.child_path
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedHudSemanticSlotIndex {
    index_identity_sha256: String,
    exact_binary_sha256: String,
    evidence_bundle_sha256: String,
    bindings: BTreeMap<AuthenticatedHudSemanticSlot, AuthenticatedHudSemanticSlotBinding>,
    identity_sha256: String,
}

impl AuthenticatedHudSemanticSlotIndex {
    pub(crate) fn binding(
        &self,
        slot: AuthenticatedHudSemanticSlot,
    ) -> Option<&AuthenticatedHudSemanticSlotBinding> {
        self.bindings.get(&slot)
    }

    pub(crate) fn identity_sha256(&self) -> &str {
        &self.identity_sha256
    }

    pub(crate) fn exact_binary_sha256(&self) -> &str {
        &self.exact_binary_sha256
    }

    pub(crate) fn evidence_bundle_sha256(&self) -> &str {
        &self.evidence_bundle_sha256
    }

    pub(crate) fn remains_bound_to(
        &self,
        panel: &StrictAuthoredPanelAdmission,
        slot: AuthenticatedHudSemanticSlot,
    ) -> bool {
        let Some(binding) = self.bindings.get(&slot) else {
            return false;
        };
        self.index_identity_sha256 == panel.index_identity_sha256()
            && binding.index_identity_sha256 == panel.index_identity_sha256()
            && binding.source_content == *panel.source_content()
            && binding.surface_id == panel.surface_id()
            && binding.authored_tree_identity_sha256 == panel.authored_tree_identity_sha256()
            && panel
                .element_role(&binding.child_path)
                .is_some_and(|role| role.authored_name == binding.engine_lookup_token)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AuthenticatedSemanticProviderRejection {
    CrossIndexPersistentHud,
    CrossIndexTheatreTemplate(AuthenticatedTheatreTemplateKind),
    DuplicateSlot(AuthenticatedHudSemanticSlot),
    DuplicateTheatreTemplate(AuthenticatedTheatreTemplateKind),
    MissingTheatreTemplate {
        kind: AuthenticatedTheatreTemplateKind,
        token: String,
    },
    MissingControllerLookup {
        slot: AuthenticatedHudSemanticSlot,
        token: String,
    },
    DuplicateControllerLookup {
        slot: AuthenticatedHudSemanticSlot,
        token: String,
    },
    InvalidEvidenceIdentity,
    InvalidSemanticValue(AuthenticatedHudSemanticSlot),
    InvalidMapmodeSelection(u8),
    MissingSealedSlot(AuthenticatedHudSemanticSlot),
    StaleActionAuthority,
    ActionLeaseMismatch,
    DuplicateActionTarget,
    InvalidPersistentHudSurface,
}

impl fmt::Display for AuthenticatedSemanticProviderRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CrossIndexPersistentHud => {
                write!(
                    f,
                    "persistent HUD panels do not share one role-index identity"
                )
            }
            Self::CrossIndexTheatreTemplate(kind) => {
                write!(
                    f,
                    "theatre template {kind:?} crosses authenticated content identity"
                )
            }
            Self::DuplicateSlot(slot) => write!(f, "duplicate semantic slot {slot:?}"),
            Self::DuplicateTheatreTemplate(kind) => {
                write!(f, "duplicate exact theatre template {kind:?}")
            }
            Self::MissingTheatreTemplate { kind, token } => write!(
                f,
                "exact theatre template {kind:?} has no admitted root {token:?}"
            ),
            Self::MissingControllerLookup { slot, token } => {
                write!(
                    f,
                    "controller slot {slot:?} has no admitted element {token:?}"
                )
            }
            Self::DuplicateControllerLookup { slot, token } => write!(
                f,
                "controller slot {slot:?} resolves admitted element {token:?} more than once"
            ),
            Self::InvalidEvidenceIdentity => {
                write!(f, "exact-a729 semantic-slot evidence identity is invalid")
            }
            Self::InvalidSemanticValue(slot) => {
                write!(
                    f,
                    "semantic value for exact controller slot {slot:?} is invalid"
                )
            }
            Self::InvalidMapmodeSelection(ordinal) => write!(
                f,
                "persistent map-mode selection {ordinal} is outside exact controller slots 1..=11"
            ),
            Self::MissingSealedSlot(slot) => {
                write!(
                    f,
                    "exact controller slot {slot:?} is absent from the sealed index"
                )
            }
            Self::StaleActionAuthority => {
                write!(
                    f,
                    "static HUD actions were requested for stale frame authority"
                )
            }
            Self::ActionLeaseMismatch => {
                write!(
                    f,
                    "static HUD action lease does not own its authenticated panel"
                )
            }
            Self::DuplicateActionTarget => {
                write!(
                    f,
                    "two exact viewer actions resolve to the same rendered target"
                )
            }
            Self::InvalidPersistentHudSurface => {
                write!(
                    f,
                    "persistent HUD surface set is not the exact controller-owned set"
                )
            }
        }
    }
}

pub(crate) struct DerivedPersistentHudPanelBindings {
    pub(crate) window_id: String,
    pub(crate) semantics: BTreeMap<String, ReplayAuthenticatedElementSemantic>,
    pub(crate) pointer_bindings: BTreeMap<String, ReplayElementPointerAuthority>,
    pub(crate) viewer_actions: Vec<ReplayViewerActionBinding>,
}

pub(crate) fn derive_exact_a729_persistent_hud_bindings(
    surfaces: &[AuthenticatedPanelSurface],
    hud: &HudBinding,
    theatre_available: bool,
) -> Result<Vec<DerivedPersistentHudPanelBindings>, AuthenticatedSemanticProviderRejection> {
    let panels = authenticate_persistent_hud_surfaces(surfaces)
        .map_err(|_| AuthenticatedSemanticProviderRejection::InvalidPersistentHudSurface)?;
    let slots = seal_exact_a729_hud_semantic_slots(&panels)?;
    let projection = project_exact_a729_hud_semantics(&slots, hud, theatre_available)?;
    let mut result = Vec::with_capacity(surfaces.len());
    for surface in surfaces {
        let suppress_theatre_surface = !theatre_available
            && surface
                .source_content()
                .normalized_game_relative_path
                .eq_ignore_ascii_case("interface/theatreselector.gui")
            && surface
                .surface_id()
                .eq_ignore_ascii_case("theatre_selection_window");
        let mut semantics_by_path = BTreeMap::new();
        let mut action_by_path = BTreeMap::new();
        for role in surface.panel.element_roles() {
            if suppress_theatre_surface {
                semantics_by_path.insert(
                    role.child_path.clone(),
                    ReplayAuthenticatedElementSemantic::Hidden,
                );
                continue;
            }
            let slot = slots
                .bindings
                .values()
                .find(|binding| {
                    binding.source_content == *surface.source_content()
                        && binding
                            .surface_id
                            .eq_ignore_ascii_case(surface.surface_id())
                        && binding.child_path == role.child_path
                })
                .map(|binding| binding.slot);
            let semantic =
                match slot.and_then(|slot| projection.value(slot).map(|value| (slot, value))) {
                    Some((slot, value)) => {
                        let semantic = projected_semantic(role, value)?;
                        if matches!(
                            &semantic,
                            ReplayAuthenticatedElementSemantic::StandardButton
                        ) {
                            if let Some(action) = static_slot_action(slot) {
                                action_by_path.insert(role.child_path.clone(), action);
                            }
                        }
                        semantic
                    }
                    None => static_authored_semantic(&surface.panel, &surface.localisation, role)?,
                };
            semantics_by_path.insert(role.child_path.clone(), semantic);
        }
        let hidden_parents = semantics_by_path
            .iter()
            .filter_map(|(path, semantic)| {
                matches!(semantic, ReplayAuthenticatedElementSemantic::Hidden)
                    .then_some(path.clone())
            })
            .collect::<Vec<_>>();
        for parent in hidden_parents {
            for (path, semantic) in semantics_by_path.iter_mut() {
                if path.len() > parent.len() && path.starts_with(&parent) {
                    if action_by_path.contains_key(path) {
                        return Err(
                            AuthenticatedSemanticProviderRejection::InvalidPersistentHudSurface,
                        );
                    }
                    *semantic = ReplayAuthenticatedElementSemantic::Hidden;
                }
            }
        }
        let mut semantics = BTreeMap::new();
        let mut pointer_bindings = BTreeMap::new();
        let mut viewer_actions = Vec::new();
        for role in surface.panel.element_roles() {
            let semantic = semantics_by_path
                .remove(&role.child_path)
                .ok_or(AuthenticatedSemanticProviderRejection::InvalidPersistentHudSurface)?;
            let action = action_by_path.remove(&role.child_path);
            let pointer = match (&semantic, action.as_ref()) {
                (ReplayAuthenticatedElementSemantic::Hidden, _) => {
                    ReplayElementPointerAuthority::PassThrough
                }
                (ReplayAuthenticatedElementSemantic::StandardButton, Some(_)) => {
                    ReplayElementPointerAuthority::Interactive {
                        visual_enabled: true,
                    }
                }
                (ReplayAuthenticatedElementSemantic::StandardButton, None) => {
                    ReplayElementPointerAuthority::BlockOnly {
                        visual_enabled: Some(true),
                    }
                }
                _ => ReplayElementPointerAuthority::PassThrough,
            };
            if let Some(action) = action {
                viewer_actions.push(ReplayViewerActionBinding::from_authenticated_owner(
                    &surface.panel,
                    role.child_path.clone(),
                    role.child_path.clone(),
                    action,
                ));
            }
            semantics.insert(role.element_id.clone(), semantic);
            pointer_bindings.insert(role.element_id.clone(), pointer);
        }
        result.push(DerivedPersistentHudPanelBindings {
            window_id: surface.window_id().to_string(),
            semantics,
            pointer_bindings,
            viewer_actions,
        });
    }
    Ok(result)
}

fn projected_semantic(
    role: &AuthoredElementRole,
    value: &AuthenticatedHudSemanticValue,
) -> Result<ReplayAuthenticatedElementSemantic, AuthenticatedSemanticProviderRejection> {
    Ok(match value {
        AuthenticatedHudSemanticValue::Hidden => ReplayAuthenticatedElementSemantic::Hidden,
        AuthenticatedHudSemanticValue::Structural => ReplayAuthenticatedElementSemantic::Structural,
        AuthenticatedHudSemanticValue::StandardButton => {
            ReplayAuthenticatedElementSemantic::StandardButton
        }
        AuthenticatedHudSemanticValue::Text(resolved_text) => {
            ReplayAuthenticatedElementSemantic::Text {
                resolved_text: resolved_text.clone(),
            }
        }
        AuthenticatedHudSemanticValue::Progress(current_state) => {
            let [sprite] = role.sprites.as_slice() else {
                return Err(AuthenticatedSemanticProviderRejection::InvalidPersistentHudSurface);
            };
            let effect = if sprite
                .effective_shader_file
                .eq_ignore_ascii_case("gfx/FX/progress.shader")
            {
                crate::widget_visual_state::ProgressEffect::Linear
            } else if sprite
                .effective_shader_file
                .eq_ignore_ascii_case("gfx/FX/progress_startend.shader")
            {
                crate::widget_visual_state::ProgressEffect::StartEnd
            } else if sprite
                .effective_shader_file
                .eq_ignore_ascii_case("gfx/FX/progress_reverse.shader")
            {
                crate::widget_visual_state::ProgressEffect::Reverse
            } else if sprite
                .effective_shader_file
                .eq_ignore_ascii_case("gfx/FX/progress_radial.shader")
            {
                crate::widget_visual_state::ProgressEffect::Radial
            } else {
                return Err(AuthenticatedSemanticProviderRejection::InvalidPersistentHudSurface);
            };
            ReplayAuthenticatedElementSemantic::Progress {
                fill: crate::widget_visual_state::ProgressFill {
                    effect,
                    current_state: *current_state,
                    circular_mix: None,
                },
            }
        }
    })
}

fn static_slot_action(slot: AuthenticatedHudSemanticSlot) -> Option<ReplayViewerAction> {
    match slot {
        AuthenticatedHudSemanticSlot::TopbarSpeedStep(step) if (1..=5).contains(&step) => Some(
            ReplayViewerAction::Transport(TransportViewerAction::SetPlaybackSpeedStep(step)),
        ),
        AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(ordinal)
        | AuthenticatedHudSemanticSlot::MapmodeButtonSelected(ordinal)
            if (1..=11).contains(&ordinal) =>
        {
            Some(ReplayViewerAction::Mapmode(
                MapmodeViewerAction::SelectMapmodeButton(ordinal),
            ))
        }
        _ => None,
    }
}

fn static_authored_semantic(
    panel: &StrictAuthoredPanelAdmission,
    localisation: &crate::loc::Localisation,
    role: &AuthoredElementRole,
) -> Result<ReplayAuthenticatedElementSemantic, AuthenticatedSemanticProviderRejection> {
    let authored = panel
        .authored_element(&role.child_path)
        .ok_or(AuthenticatedSemanticProviderRejection::InvalidPersistentHudSurface)?;
    if WidgetFamily::from_element_type(&authored.element_type)
        .is_some_and(WidgetFamily::standard_button_carrier)
        && role.sprites.len() == 1
    {
        return Ok(ReplayAuthenticatedElementSemantic::Hidden);
    }
    if role.sprites.len() == 1
        && role.direct_textures.is_empty()
        && authored.font.is_none()
        && authored.text.is_none()
    {
        return Ok(ReplayAuthenticatedElementSemantic::PassiveSprite {
            one_based_frame: role.authored_one_based_frame,
        });
    }
    if let Some(token) = authored.text.as_deref() {
        if localisation.ambiguous_sources_for_key(token).is_some() {
            return Err(AuthenticatedSemanticProviderRejection::InvalidPersistentHudSurface);
        }
        let runtime = crate::text_law::RuntimeTextContext::default();
        let resolved = if localisation.get(token).is_some() {
            crate::text_law::resolve_localisation(&localisation.entries, token, &runtime)
        } else {
            crate::text_law::resolve_template(&localisation.entries, token, &runtime)
        };
        return Ok(if resolved.admitted() {
            ReplayAuthenticatedElementSemantic::Text {
                resolved_text: resolved.text,
            }
        } else {
            ReplayAuthenticatedElementSemantic::Hidden
        });
    }
    if role.sprites.is_empty()
        && role.direct_textures.is_empty()
        && authored.font.is_none()
        && authored.text.is_none()
    {
        return Ok(ReplayAuthenticatedElementSemantic::Structural);
    }
    Ok(ReplayAuthenticatedElementSemantic::Hidden)
}

pub(crate) fn authenticated_static_panel_semantics(
    surface: &AuthenticatedPanelSurface,
) -> Result<
    BTreeMap<String, ReplayAuthenticatedElementSemantic>,
    AuthenticatedSemanticProviderRejection,
> {
    surface
        .panel
        .element_roles()
        .iter()
        .map(|role| {
            Ok((
                role.element_id.clone(),
                static_authored_semantic(&surface.panel, &surface.localisation, role)?,
            ))
        })
        .collect()
}

pub(crate) fn authenticated_theatre_template_static_semantics(
    index: &AuthenticatedGuiRoleIndex,
    admission: &ValidatedContentAdmission,
    owner_surface: &AuthenticatedPanelSurface,
    panel: &StrictAuthoredPanelAdmission,
) -> Result<
    BTreeMap<Vec<u32>, ReplayAuthenticatedElementSemantic>,
    AuthenticatedSemanticProviderRejection,
> {
    index
        .validate_panel_token(admission, panel)
        .map_err(|_| AuthenticatedSemanticProviderRejection::InvalidPersistentHudSurface)?;
    panel
        .element_roles()
        .iter()
        .map(|role| {
            Ok((
                role.child_path.clone(),
                static_authored_semantic(panel, &owner_surface.localisation, role)?,
            ))
        })
        .collect()
}

impl std::error::Error for AuthenticatedSemanticProviderRejection {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthenticatedViewerActionKind {
    SetPlaybackSpeedStep(u8),
    SelectMapmodeButton(u8),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AuthenticatedStaticActionTarget {
    window_id: String,
    runtime_instance_token: String,
    frame_generation: u64,
    child_path: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthenticatedStaticActionBinding {
    target: AuthenticatedStaticActionTarget,
    slot: AuthenticatedHudSemanticSlot,
    action: AuthenticatedViewerActionKind,
    slot_index_identity_sha256: String,
    source_content: UsedContentKey,
    authored_tree_identity_sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedStaticHudActionCensus {
    actions: BTreeMap<AuthenticatedStaticActionTarget, AuthenticatedStaticActionBinding>,
    identity_sha256: String,
}

impl AuthenticatedStaticHudActionCensus {
    pub(crate) fn resolve(
        &self,
        window_id: &str,
        runtime_instance_token: &str,
        frame_generation: u64,
        child_path: &[u32],
    ) -> Option<AuthenticatedViewerActionKind> {
        self.actions
            .get(&AuthenticatedStaticActionTarget {
                window_id: window_id.to_string(),
                runtime_instance_token: runtime_instance_token.to_string(),
                frame_generation,
                child_path: child_path.to_vec(),
            })
            .map(|binding| binding.action)
    }

    pub(crate) fn identity_sha256(&self) -> &str {
        &self.identity_sha256
    }

    pub(crate) fn len(&self) -> usize {
        self.actions.len()
    }
}

pub(crate) fn seal_exact_a729_static_hud_actions(
    manager: &ReplayOwnedWindowManager,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    panels: &AuthenticatedPersistentHudPanels,
    slots: &AuthenticatedHudSemanticSlotIndex,
    topbar_lease: &AuthenticatedReplayPanelLease<'_>,
    mapmodes_lease: &AuthenticatedReplayPanelLease<'_>,
) -> Result<AuthenticatedStaticHudActionCensus, AuthenticatedSemanticProviderRejection> {
    let topbar = &panels.panels()[0];
    let mapmodes = &panels.panels()[2];
    if !frame_authority.remains_staged_for(manager) {
        return Err(AuthenticatedSemanticProviderRejection::StaleActionAuthority);
    }
    for (panel, lease) in [(topbar, topbar_lease), (mapmodes, mapmodes_lease)] {
        validate_static_action_lease(
            lease.remains_current(),
            lease.window_id(),
            panel.window_id(),
            lease.runtime_instance_token(),
            panel.index_identity_sha256(),
            &slots.index_identity_sha256,
        )?;
    }
    let mut actions = BTreeMap::new();
    for step in 1..=5 {
        insert_static_action(
            &mut actions,
            frame_authority,
            topbar,
            topbar_lease,
            slots,
            AuthenticatedHudSemanticSlot::TopbarSpeedStep(step),
            AuthenticatedViewerActionKind::SetPlaybackSpeedStep(step),
        )?;
    }
    for ordinal in 1..=11 {
        for slot in [
            AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(ordinal),
            AuthenticatedHudSemanticSlot::MapmodeButtonSelected(ordinal),
        ] {
            insert_static_action(
                &mut actions,
                frame_authority,
                mapmodes,
                mapmodes_lease,
                slots,
                slot,
                AuthenticatedViewerActionKind::SelectMapmodeButton(ordinal),
            )?;
        }
    }
    let identity_sha256 = static_action_census_digest(&actions);
    Ok(AuthenticatedStaticHudActionCensus {
        actions,
        identity_sha256,
    })
}

fn validate_static_action_lease(
    lease_current: bool,
    lease_window_id: &str,
    panel_window_id: &str,
    runtime_instance_token: &str,
    panel_index_identity_sha256: &str,
    slot_index_identity_sha256: &str,
) -> Result<(), AuthenticatedSemanticProviderRejection> {
    if !lease_current
        || lease_window_id != panel_window_id
        || runtime_instance_token.trim().is_empty()
        || panel_index_identity_sha256 != slot_index_identity_sha256
    {
        return Err(AuthenticatedSemanticProviderRejection::ActionLeaseMismatch);
    }
    Ok(())
}

fn insert_static_action(
    actions: &mut BTreeMap<AuthenticatedStaticActionTarget, AuthenticatedStaticActionBinding>,
    frame_authority: &AuthenticatedReplayFrameAuthority,
    panel: &StrictAuthoredPanelAdmission,
    lease: &AuthenticatedReplayPanelLease<'_>,
    slots: &AuthenticatedHudSemanticSlotIndex,
    slot: AuthenticatedHudSemanticSlot,
    action: AuthenticatedViewerActionKind,
) -> Result<(), AuthenticatedSemanticProviderRejection> {
    let binding =
        slots
            .binding(slot)
            .ok_or(AuthenticatedSemanticProviderRejection::MissingSealedSlot(
                slot,
            ))?;
    if !slots.remains_bound_to(panel, slot) {
        return Err(AuthenticatedSemanticProviderRejection::ActionLeaseMismatch);
    }
    let target = AuthenticatedStaticActionTarget {
        window_id: panel.window_id().to_string(),
        runtime_instance_token: lease.runtime_instance_token().to_string(),
        frame_generation: frame_authority.frame_generation(),
        child_path: binding.child_path.clone(),
    };
    let action_binding = AuthenticatedStaticActionBinding {
        target: target.clone(),
        slot,
        action,
        slot_index_identity_sha256: slots.identity_sha256().to_string(),
        source_content: binding.source_content.clone(),
        authored_tree_identity_sha256: binding.authored_tree_identity_sha256.clone(),
    };
    insert_static_action_binding(actions, target, action_binding)
}

fn insert_static_action_binding(
    actions: &mut BTreeMap<AuthenticatedStaticActionTarget, AuthenticatedStaticActionBinding>,
    target: AuthenticatedStaticActionTarget,
    action_binding: AuthenticatedStaticActionBinding,
) -> Result<(), AuthenticatedSemanticProviderRejection> {
    if action_binding.target != target || actions.insert(target, action_binding).is_some() {
        return Err(AuthenticatedSemanticProviderRejection::DuplicateActionTarget);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AuthenticatedHudSemanticValue {
    Hidden,
    Structural,
    StandardButton,
    Text(String),
    Progress(f32),
}

#[derive(Debug, PartialEq)]
pub(crate) struct AuthenticatedHudSemanticProjection {
    slot_index_identity_sha256: String,
    values: BTreeMap<AuthenticatedHudSemanticSlot, AuthenticatedHudSemanticValue>,
    identity_sha256: String,
}

impl AuthenticatedHudSemanticProjection {
    pub(crate) fn value(
        &self,
        slot: AuthenticatedHudSemanticSlot,
    ) -> Option<&AuthenticatedHudSemanticValue> {
        self.values.get(&slot)
    }

    pub(crate) fn identity_sha256(&self) -> &str {
        &self.identity_sha256
    }

    pub(crate) fn remains_bound_to(&self, slots: &AuthenticatedHudSemanticSlotIndex) -> bool {
        self.slot_index_identity_sha256 == slots.identity_sha256()
            && self.values.len() == slots.bindings.len()
            && self
                .values
                .keys()
                .all(|slot| slots.binding(*slot).is_some())
            && self.identity_sha256
                == semantic_projection_digest(slots.identity_sha256(), &self.values)
    }
}

pub(crate) fn project_exact_a729_hud_semantics(
    slots: &AuthenticatedHudSemanticSlotIndex,
    hud: &HudBinding,
    theatre_available: bool,
) -> Result<AuthenticatedHudSemanticProjection, AuthenticatedSemanticProviderRejection> {
    if slots.exact_binary_sha256 != EXACT_A729_BINARY_SHA256
        || slots.evidence_bundle_sha256 != CONTROLLER_EVIDENCE_BUNDLE_SHA256
    {
        return Err(AuthenticatedSemanticProviderRejection::InvalidEvidenceIdentity);
    }
    if hud.mapmode.selected_button > 11 {
        return Err(
            AuthenticatedSemanticProviderRejection::InvalidMapmodeSelection(
                hud.mapmode.selected_button,
            ),
        );
    }
    if hud.mapmode.selected_button != 0 && hud.mapmode.use_four_button_layout.is_none() {
        return Err(
            AuthenticatedSemanticProviderRejection::InvalidMapmodeSelection(
                hud.mapmode.selected_button,
            ),
        );
    }

    let mut values = BTreeMap::new();
    insert_slot(
        slots,
        &mut values,
        AuthenticatedHudSemanticSlot::TopbarPlayerFlag,
        if hud.topbar.country_tag.is_some() {
            AuthenticatedHudSemanticValue::StandardButton
        } else {
            AuthenticatedHudSemanticValue::Hidden
        },
    )?;
    for (slot, value) in [
        (
            AuthenticatedHudSemanticSlot::TopbarPoliticalPower,
            hud.topbar.political_power.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarManpower,
            hud.topbar.manpower.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarStability,
            hud.topbar.stability_percent.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarWarSupport,
            hud.topbar.war_support_percent.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarCommandPower,
            hud.topbar.command_power.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarArmyExperience,
            hud.topbar.army_experience.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarNavyExperience,
            hud.topbar.navy_experience.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarAirExperience,
            hud.topbar.air_experience.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarNukes,
            hud.topbar.nukes.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarIndustrialCapacity,
            hud.topbar.factory_text.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarConvoys,
            hud.topbar.convoys.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarFuel,
            hud.topbar.fuel.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarSupply,
            hud.topbar.supply.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarThreatValue,
            hud.topbar.world_tension_percent.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarDateText,
            hud.topbar.date_time.as_deref(),
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarInitiative,
            hud.topbar.initiative.as_deref(),
        ),
    ] {
        insert_slot(
            slots,
            &mut values,
            slot,
            value
                .filter(|value| !value.is_empty())
                .map(|value| AuthenticatedHudSemanticValue::Text(value.to_string()))
                .unwrap_or(AuthenticatedHudSemanticValue::Hidden),
        )?;
    }
    for (slot, value) in [
        (
            AuthenticatedHudSemanticSlot::TopbarIndustrialRatio,
            hud.topbar.industrial_ratio,
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarConvoysRatio,
            hud.topbar.convoys_ratio,
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarFuelRatio,
            hud.topbar.fuel_ratio,
        ),
        (
            AuthenticatedHudSemanticSlot::TopbarSupplyRatio,
            hud.topbar.supply_ratio,
        ),
    ] {
        let semantic = match value {
            Some(value) if value.is_finite() && (0.0..=1.0).contains(&value) => {
                AuthenticatedHudSemanticValue::Progress(value)
            }
            None => AuthenticatedHudSemanticValue::Hidden,
            Some(_) => {
                return Err(AuthenticatedSemanticProviderRejection::InvalidSemanticValue(slot))
            }
        };
        insert_slot(slots, &mut values, slot, semantic)?;
    }
    let speed = hud.topbar.speed.filter(|step| (1..=5).contains(step));
    if hud.topbar.speed.is_some() && speed.is_none() {
        return Err(
            AuthenticatedSemanticProviderRejection::InvalidSemanticValue(
                AuthenticatedHudSemanticSlot::TopbarSpeedStep(hud.topbar.speed.unwrap_or_default()),
            ),
        );
    }
    for step in 1..=5 {
        insert_slot(
            slots,
            &mut values,
            AuthenticatedHudSemanticSlot::TopbarSpeedStep(step),
            AuthenticatedHudSemanticValue::StandardButton,
        )?;
    }
    let four = hud.mapmode.use_four_button_layout;
    insert_slot(
        slots,
        &mut values,
        AuthenticatedHudSemanticSlot::MapmodeThreeButtonLayout,
        if four == Some(false) {
            AuthenticatedHudSemanticValue::Structural
        } else {
            AuthenticatedHudSemanticValue::Hidden
        },
    )?;
    insert_slot(
        slots,
        &mut values,
        AuthenticatedHudSemanticSlot::MapmodeFourButtonLayout,
        if four == Some(true) {
            AuthenticatedHudSemanticValue::Structural
        } else {
            AuthenticatedHudSemanticValue::Hidden
        },
    )?;
    for ordinal in 1..=11 {
        let selected = hud.mapmode.selected_button == ordinal;
        insert_slot(
            slots,
            &mut values,
            AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(ordinal),
            if selected {
                AuthenticatedHudSemanticValue::Hidden
            } else {
                AuthenticatedHudSemanticValue::StandardButton
            },
        )?;
        insert_slot(
            slots,
            &mut values,
            AuthenticatedHudSemanticSlot::MapmodeButtonSelected(ordinal),
            if selected {
                AuthenticatedHudSemanticValue::StandardButton
            } else {
                AuthenticatedHudSemanticValue::Hidden
            },
        )?;
    }
    insert_slot(
        slots,
        &mut values,
        AuthenticatedHudSemanticSlot::TheatreElementsList,
        if theatre_available {
            AuthenticatedHudSemanticValue::Structural
        } else {
            AuthenticatedHudSemanticValue::Hidden
        },
    )?;
    let identity_sha256 = semantic_projection_digest(slots.identity_sha256(), &values);
    Ok(AuthenticatedHudSemanticProjection {
        slot_index_identity_sha256: slots.identity_sha256().to_string(),
        values,
        identity_sha256,
    })
}

fn insert_slot(
    slots: &AuthenticatedHudSemanticSlotIndex,
    values: &mut BTreeMap<AuthenticatedHudSemanticSlot, AuthenticatedHudSemanticValue>,
    slot: AuthenticatedHudSemanticSlot,
    value: AuthenticatedHudSemanticValue,
) -> Result<(), AuthenticatedSemanticProviderRejection> {
    if slots.binding(slot).is_none() {
        return Err(AuthenticatedSemanticProviderRejection::MissingSealedSlot(
            slot,
        ));
    }
    if values.insert(slot, value).is_some() {
        return Err(AuthenticatedSemanticProviderRejection::DuplicateSlot(slot));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AuthenticatedTheatreTemplateKind {
    SettingsView,
    OrdersGroup,
    NavyFleet,
    LandTheatre,
    NavyTheatre,
    ReserveNavyTheatre,
    NewTheatre,
    StrategicRegionAlert,
    StrategicRegion,
    ArmyGroupBox,
    FleetRowBox,
    ReserveFleetRowBox,
    FreeAirWings,
    AirWing,
    AirTheatre,
    AirTheatreGroupRow,
    AirTheatreGroup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExactA729TheatreTemplateClaim {
    kind: AuthenticatedTheatreTemplateKind,
    root_token: &'static str,
    controller_rva: u32,
    evidence_job_id: &'static str,
    evidence_artifact_sha256: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthenticatedTheatreTemplateSet {
    index_identity_sha256: String,
    source_content: UsedContentKey,
    templates: BTreeMap<AuthenticatedTheatreTemplateKind, StrictAuthoredPanelAdmission>,
    identity_sha256: String,
}

impl AuthenticatedTheatreTemplateSet {
    pub(crate) fn identity_sha256(&self) -> &str {
        &self.identity_sha256
    }

    pub(crate) fn contains(&self, kind: AuthenticatedTheatreTemplateKind) -> bool {
        self.templates.contains_key(&kind)
    }

    pub(crate) fn template(
        &self,
        kind: AuthenticatedTheatreTemplateKind,
    ) -> Option<&StrictAuthoredPanelAdmission> {
        self.templates.get(&kind)
    }
}

pub(crate) fn seal_exact_a729_theatre_templates(
    panels: &AuthenticatedPersistentHudPanels,
    index: &AuthenticatedGuiRoleIndex,
    catalog: &GuiDefinitionCatalog,
    admission: &ValidatedContentAdmission,
) -> Result<AuthenticatedTheatreTemplateSet, AuthenticatedSemanticProviderRejection> {
    let first_index = panels.panels()[0].index_identity_sha256();
    if index.index_identity_sha256 != first_index
        || panels
            .panels()
            .iter()
            .any(|panel| panel.index_identity_sha256() != first_index)
    {
        return Err(AuthenticatedSemanticProviderRejection::CrossIndexPersistentHud);
    }
    let source_content = admission
        .used_content("interface/theatreselector.gui")
        .ok_or_else(
            || AuthenticatedSemanticProviderRejection::MissingTheatreTemplate {
                kind: AuthenticatedTheatreTemplateKind::LandTheatre,
                token: "interface/theatreselector.gui".to_string(),
            },
        )?;
    let mut templates = BTreeMap::new();
    for claim in exact_a729_theatre_template_claims() {
        if !valid_sha256(claim.evidence_artifact_sha256) {
            return Err(AuthenticatedSemanticProviderRejection::InvalidEvidenceIdentity);
        }
        let panel = index
            .admit_surface(catalog, admission, source_content, claim.root_token)
            .map_err(
                |_| AuthenticatedSemanticProviderRejection::MissingTheatreTemplate {
                    kind: claim.kind,
                    token: claim.root_token.to_string(),
                },
            )?;
        let rebound = index.validate_panel_token(admission, &panel).map_err(|_| {
            AuthenticatedSemanticProviderRejection::MissingTheatreTemplate {
                kind: claim.kind,
                token: claim.root_token.to_string(),
            }
        })?;
        if panel.index_identity_sha256() != first_index
            || panel.source_content() != source_content
            || !panel.surface_id().eq_ignore_ascii_case(claim.root_token)
            || !rebound
                .source_file
                .eq_ignore_ascii_case("interface/theatreselector.gui")
        {
            return Err(
                AuthenticatedSemanticProviderRejection::CrossIndexTheatreTemplate(claim.kind),
            );
        }
        if templates.insert(claim.kind, panel).is_some() {
            return Err(
                AuthenticatedSemanticProviderRejection::DuplicateTheatreTemplate(claim.kind),
            );
        }
    }
    let identity_sha256 = theatre_template_set_digest(first_index, source_content, &templates);
    Ok(AuthenticatedTheatreTemplateSet {
        index_identity_sha256: first_index.to_string(),
        source_content: source_content.clone(),
        templates,
        identity_sha256,
    })
}

pub(crate) fn seal_exact_a729_hud_semantic_slots(
    panels: &AuthenticatedPersistentHudPanels,
) -> Result<AuthenticatedHudSemanticSlotIndex, AuthenticatedSemanticProviderRejection> {
    if !valid_sha256(EXACT_A729_BINARY_SHA256) || !valid_sha256(CONTROLLER_EVIDENCE_BUNDLE_SHA256) {
        return Err(AuthenticatedSemanticProviderRejection::InvalidEvidenceIdentity);
    }
    let persistent = panels.panels();
    let first_index = persistent[0].index_identity_sha256();
    if persistent
        .iter()
        .any(|panel| panel.index_identity_sha256() != first_index)
    {
        return Err(AuthenticatedSemanticProviderRejection::CrossIndexPersistentHud);
    }

    seal_slot_index(
        first_index,
        |panel| match panel {
            ControllerPanel::Topbar => &persistent[0],
            ControllerPanel::Mapmodes => &persistent[2],
            ControllerPanel::TheatreRoot => &persistent[3],
        },
        exact_a729_controller_claims(),
    )
}

fn seal_slot_index<'a>(
    index_identity_sha256: &str,
    panel: impl Fn(ControllerPanel) -> &'a StrictAuthoredPanelAdmission,
    claims: impl IntoIterator<Item = ExactA729ControllerClaim>,
) -> Result<AuthenticatedHudSemanticSlotIndex, AuthenticatedSemanticProviderRejection> {
    if !valid_sha256(index_identity_sha256) {
        return Err(AuthenticatedSemanticProviderRejection::CrossIndexPersistentHud);
    }
    let mut bindings = BTreeMap::new();
    for claim in claims {
        if !valid_sha256(claim.evidence_artifact_sha256) {
            return Err(AuthenticatedSemanticProviderRejection::InvalidEvidenceIdentity);
        }
        let owner = panel(claim.panel);
        if owner.index_identity_sha256() != index_identity_sha256 {
            return Err(AuthenticatedSemanticProviderRejection::CrossIndexPersistentHud);
        }
        let matches = owner
            .element_roles()
            .iter()
            .filter(|role| role.authored_name == claim.lookup_token)
            .collect::<Vec<_>>();
        let role = match matches.as_slice() {
            [] => {
                return Err(
                    AuthenticatedSemanticProviderRejection::MissingControllerLookup {
                        slot: claim.slot,
                        token: claim.lookup_token.to_string(),
                    },
                )
            }
            [role] => *role,
            _ => {
                return Err(
                    AuthenticatedSemanticProviderRejection::DuplicateControllerLookup {
                        slot: claim.slot,
                        token: claim.lookup_token.to_string(),
                    },
                )
            }
        };
        let binding = binding_from_claim(index_identity_sha256, owner, role, claim);
        if bindings.insert(claim.slot, binding).is_some() {
            return Err(AuthenticatedSemanticProviderRejection::DuplicateSlot(
                claim.slot,
            ));
        }
    }
    let identity_sha256 = slot_index_digest(index_identity_sha256, &bindings);
    Ok(AuthenticatedHudSemanticSlotIndex {
        index_identity_sha256: index_identity_sha256.to_string(),
        exact_binary_sha256: EXACT_A729_BINARY_SHA256.to_string(),
        evidence_bundle_sha256: CONTROLLER_EVIDENCE_BUNDLE_SHA256.to_string(),
        bindings,
        identity_sha256,
    })
}

fn binding_from_claim(
    index_identity_sha256: &str,
    owner: &StrictAuthoredPanelAdmission,
    role: &AuthoredElementRole,
    claim: ExactA729ControllerClaim,
) -> AuthenticatedHudSemanticSlotBinding {
    AuthenticatedHudSemanticSlotBinding {
        slot: claim.slot,
        index_identity_sha256: index_identity_sha256.to_string(),
        source_content: owner.source_content().clone(),
        surface_id: owner.surface_id().to_string(),
        authored_tree_identity_sha256: owner.authored_tree_identity_sha256().to_string(),
        child_path: role.child_path.clone(),
        engine_lookup_token: claim.lookup_token.to_string(),
        controller_rva: claim.controller_rva,
        evidence_job_id: claim.evidence_job_id.to_string(),
        evidence_artifact_sha256: claim.evidence_artifact_sha256.to_string(),
    }
}

fn slot_index_digest(
    index_identity_sha256: &str,
    bindings: &BTreeMap<AuthenticatedHudSemanticSlot, AuthenticatedHudSemanticSlotBinding>,
) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, index_identity_sha256);
    push_string(&mut bytes, EXACT_A729_BINARY_SHA256);
    push_string(&mut bytes, CONTROLLER_EVIDENCE_BUNDLE_SHA256);
    for (slot, binding) in bindings {
        push_string(&mut bytes, &format!("{slot:?}"));
        push_string(
            &mut bytes,
            &binding.source_content.effective_vfs_manifest_sha256,
        );
        push_string(
            &mut bytes,
            &binding.source_content.normalized_game_relative_path,
        );
        bytes.extend_from_slice(&binding.source_content.winning_layer_ordinal.to_le_bytes());
        push_string(
            &mut bytes,
            &format!("{:?}", binding.source_content.layer_kind),
        );
        push_string(&mut bytes, &binding.source_content.layer_id);
        push_string(&mut bytes, &binding.source_content.source_sha256);
        push_string(&mut bytes, &binding.source_content.payload_sha256);
        bytes.extend_from_slice(&binding.source_content.payload_bytes.to_le_bytes());
        push_string(&mut bytes, &binding.surface_id);
        push_string(&mut bytes, &binding.authored_tree_identity_sha256);
        bytes.extend_from_slice(&(binding.child_path.len() as u64).to_le_bytes());
        for ordinal in &binding.child_path {
            bytes.extend_from_slice(&ordinal.to_le_bytes());
        }
        push_string(&mut bytes, &binding.engine_lookup_token);
        bytes.extend_from_slice(&binding.controller_rva.to_le_bytes());
        push_string(&mut bytes, &binding.evidence_job_id);
        push_string(&mut bytes, &binding.evidence_artifact_sha256);
    }
    Sha256::hex(&bytes)
}

fn theatre_template_set_digest(
    index_identity_sha256: &str,
    source_content: &UsedContentKey,
    templates: &BTreeMap<AuthenticatedTheatreTemplateKind, StrictAuthoredPanelAdmission>,
) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, index_identity_sha256);
    push_string(&mut bytes, EXACT_A729_BINARY_SHA256);
    push_string(&mut bytes, CONTROLLER_EVIDENCE_BUNDLE_SHA256);
    push_string(&mut bytes, &source_content.effective_vfs_manifest_sha256);
    push_string(&mut bytes, &source_content.normalized_game_relative_path);
    push_string(&mut bytes, &source_content.source_sha256);
    for (kind, panel) in templates {
        push_string(&mut bytes, &format!("{kind:?}"));
        push_string(&mut bytes, panel.surface_id());
        push_string(&mut bytes, panel.authored_tree_identity_sha256());
        let claim = exact_a729_theatre_template_claims()
            .into_iter()
            .find(|claim| claim.kind == *kind)
            .expect("sealed theatre template kind has an exact claim");
        bytes.extend_from_slice(&claim.controller_rva.to_le_bytes());
        push_string(&mut bytes, claim.evidence_job_id);
        push_string(&mut bytes, claim.evidence_artifact_sha256);
    }
    Sha256::hex(&bytes)
}

fn semantic_projection_digest(
    slot_index_identity_sha256: &str,
    values: &BTreeMap<AuthenticatedHudSemanticSlot, AuthenticatedHudSemanticValue>,
) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, slot_index_identity_sha256);
    for (slot, value) in values {
        push_string(&mut bytes, &format!("{slot:?}"));
        match value {
            AuthenticatedHudSemanticValue::Hidden => bytes.push(0),
            AuthenticatedHudSemanticValue::Structural => bytes.push(1),
            AuthenticatedHudSemanticValue::StandardButton => bytes.push(2),
            AuthenticatedHudSemanticValue::Text(value) => {
                bytes.push(3);
                push_string(&mut bytes, value);
            }
            AuthenticatedHudSemanticValue::Progress(value) => {
                bytes.push(4);
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
    }
    Sha256::hex(&bytes)
}

fn static_action_census_digest(
    actions: &BTreeMap<AuthenticatedStaticActionTarget, AuthenticatedStaticActionBinding>,
) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, EXACT_A729_BINARY_SHA256);
    push_string(&mut bytes, CONTROLLER_EVIDENCE_BUNDLE_SHA256);
    for (target, binding) in actions {
        push_string(&mut bytes, &target.window_id);
        push_string(&mut bytes, &target.runtime_instance_token);
        bytes.extend_from_slice(&target.frame_generation.to_le_bytes());
        bytes.extend_from_slice(&(target.child_path.len() as u64).to_le_bytes());
        for ordinal in &target.child_path {
            bytes.extend_from_slice(&ordinal.to_le_bytes());
        }
        push_string(&mut bytes, &format!("{:?}", binding.slot));
        push_string(&mut bytes, &format!("{:?}", binding.action));
        push_string(&mut bytes, &binding.slot_index_identity_sha256);
        push_string(&mut bytes, &binding.source_content.source_sha256);
        push_string(&mut bytes, &binding.authored_tree_identity_sha256);
    }
    Sha256::hex(&bytes)
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn exact_a729_controller_claims() -> Vec<ExactA729ControllerClaim> {
    const TOPBAR_INIT_JOB: &str = "20260729T074136Z-xui07-topbar-runtime-slot-controller-89a9e58b";
    const TOPBAR_INIT_SHA: &str =
        "ac85a9ade9ec5c664b7be8d26e34c4229d3577070c1af3473f928ca39acb4a1b";
    const TOPBAR_SETUP_JOB: &str = "20260729T074050Z-xui07-provider-controller-slot-proof-a4515c48";
    const TOPBAR_SETUP_SHA: &str =
        "543b6ef4b167e05e7b1f1f0937019943740fd0b1f6fe3dde1b8c5f9ff26f2243";
    const TOPBAR_ALERT_A_JOB: &str =
        "20260729T084941Z-xui07-complete-persistent-hud-controller-68cd4289";
    const TOPBAR_ALERT_A_SHA: &str =
        "4271c53240ac60f1f40d1656055198473ec89a5d4c3be558956b4f974bbead75";
    const MAPMODE_JOB: &str = "20260729T074050Z-xui07-provider-controller-slot-proof-a4515c48";
    const MAPMODE_SHA: &str = "3cdb630c06280a2bf3a9775fe7eaf7b2b9b99e47eafcc68e6b2035cef64b71cf";
    const MAPMODE_TWIN_JOB: &str =
        "20260729T081939Z-xui07-mapmode-persistent-selected-state--df80b690";
    const MAPMODE_TWIN_SETUP_SHA: &str =
        "3cdb630c06280a2bf3a9775fe7eaf7b2b9b99e47eafcc68e6b2035cef64b71cf";
    const THEATRE_SETUP_JOB: &str =
        "20260729T074050Z-xui07-provider-controller-slot-proof-a4515c48";
    const THEATRE_SETUP_SHA: &str =
        "fa8de0e2644ebc72cca3695835cc98395b546a49ab50c14d0a1786eddc6a1bee";
    let mut claims = vec![
        claim(
            AuthenticatedHudSemanticSlot::TopbarPlayerFlag,
            ControllerPanel::Topbar,
            "player_flag",
            0x0188_ba30,
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarPoliticalPower,
            "pol_power",
            TOPBAR_INIT_JOB,
            TOPBAR_INIT_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarManpower,
            "manpower",
            TOPBAR_INIT_JOB,
            TOPBAR_INIT_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarStability,
            "stability",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarWarSupport,
            "war_support",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarCommandPower,
            "command_power_value",
            TOPBAR_INIT_JOB,
            TOPBAR_INIT_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarArmyExperience,
            "experience_value",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarNavyExperience,
            "navy_experience_value",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarAirExperience,
            "air_experience_value",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarNukes,
            "nukes_value",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarIndustrialCapacity,
            "industrial_capacity",
            TOPBAR_INIT_JOB,
            TOPBAR_INIT_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarIndustrialRatio,
            "industrial_ratio_bar",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarConvoys,
            "convoys_count",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarConvoysRatio,
            "convoys_ratio_bar",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarFuel,
            "fuel_value",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarFuelRatio,
            "fuel_ratio_bar",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarSupply,
            "supply_value",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarSupplyRatio,
            "supply_ratio_bar",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarThreatValue,
            "threat_value",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        topbar_claim(
            AuthenticatedHudSemanticSlot::TopbarDateText,
            "DateText",
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ),
        claim(
            AuthenticatedHudSemanticSlot::TopbarInitiative,
            ControllerPanel::Topbar,
            "initiative_text",
            0x01e6_1910,
            TOPBAR_ALERT_A_JOB,
            TOPBAR_ALERT_A_SHA,
        ),
        claim(
            AuthenticatedHudSemanticSlot::MapmodeThreeButtonLayout,
            ControllerPanel::Mapmodes,
            "3_button_mapmode",
            0x0156_1c20,
            MAPMODE_JOB,
            MAPMODE_SHA,
        ),
        claim(
            AuthenticatedHudSemanticSlot::MapmodeFourButtonLayout,
            ControllerPanel::Mapmodes,
            "4_button_mapmode",
            0x0156_1c20,
            MAPMODE_JOB,
            MAPMODE_SHA,
        ),
        claim(
            AuthenticatedHudSemanticSlot::TheatreElementsList,
            ControllerPanel::TheatreRoot,
            "elements_list",
            0x0187_8e40,
            THEATRE_SETUP_JOB,
            THEATRE_SETUP_SHA,
        ),
    ];
    for step in 1..=5 {
        claims.push(claim(
            AuthenticatedHudSemanticSlot::TopbarSpeedStep(step),
            ControllerPanel::Topbar,
            match step {
                1 => "button_speedstep1",
                2 => "button_speedstep2",
                3 => "button_speedstep3",
                4 => "button_speedstep4",
                5 => "button_speedstep5",
                _ => unreachable!(),
            },
            0x0188_ba30,
            TOPBAR_SETUP_JOB,
            TOPBAR_SETUP_SHA,
        ));
    }
    for ordinal in 1..=11 {
        let deselected_token = match ordinal {
            1 => "mapmode_button_1",
            2 => "mapmode_button_2",
            3 => "mapmode_button_3",
            4 => "mapmode_button_4",
            5 => "mapmode_button_5",
            6 => "mapmode_button_6",
            7 => "mapmode_button_7",
            8 => "mapmode_button_8",
            9 => "mapmode_button_9",
            10 => "mapmode_button_10",
            11 => "mapmode_button_11",
            _ => unreachable!(),
        };
        let selected_token = match ordinal {
            1 => "mapmode_button_1_s",
            2 => "mapmode_button_2_s",
            3 => "mapmode_button_3_s",
            4 => "mapmode_button_4_s",
            5 => "mapmode_button_5_s",
            6 => "mapmode_button_6_s",
            7 => "mapmode_button_7_s",
            8 => "mapmode_button_8_s",
            9 => "mapmode_button_9_s",
            10 => "mapmode_button_10_s",
            11 => "mapmode_button_11_s",
            _ => unreachable!(),
        };
        claims.push(claim(
            AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(ordinal),
            ControllerPanel::Mapmodes,
            deselected_token,
            0x0156_3c80,
            MAPMODE_TWIN_JOB,
            MAPMODE_TWIN_SETUP_SHA,
        ));
        claims.push(claim(
            AuthenticatedHudSemanticSlot::MapmodeButtonSelected(ordinal),
            ControllerPanel::Mapmodes,
            selected_token,
            0x0156_3c80,
            MAPMODE_TWIN_JOB,
            MAPMODE_TWIN_SETUP_SHA,
        ));
    }
    claims
}

fn exact_a729_theatre_template_claims() -> Vec<ExactA729TheatreTemplateClaim> {
    const JOB_A: &str = "20260729T082249Z-xui07-actual-theatre-template-controller-8d6a596a";
    const SHA_A: &str = "2ff25841dcb0a7cff7b6eb4a40191f44f68ec353130a6bd58fe00e1f4fceaf15";
    const JOB_B: &str = "20260729T082255Z-xui07-actual-theatre-template-controller-863f94d2";
    const SHA_B: &str = "a41f07b8d1725c39d825fc38aa92180eb5b21e3f94ca7daa4df3245d6b0a1f6a";
    vec![
        theatre_claim(
            AuthenticatedTheatreTemplateKind::SettingsView,
            "theater_group_settings_view",
            0x01f8_6ff0,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::OrdersGroup,
            "orders_group_item",
            0x01e5_07e0,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::NavyFleet,
            "navy_theater_fleet_item",
            0x0008_17a0,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::LandTheatre,
            "theater_group_item",
            0x01e5_17d0,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::NavyTheatre,
            "navy_theater_group_item",
            0x0008_1980,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::ReserveNavyTheatre,
            "reserve_navy_theater_group_item",
            0x0187_a3b0,
            "20260729T123624Z-xui07-reserve-navy-template-inactive-law-7831ba4e",
            "37769949b4cbf6146032733af5a5883f78738901d7effdf4ea5ec69d54279e9a",
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::NewTheatre,
            "new_theater_group_item",
            0x0187_8e40,
            "20260729T122152Z-xui07-new-theatre-unconditional-presence-2aaf2d3a",
            "fa8de0e2644ebc72cca3695835cc98395b546a49ab50c14d0a1786eddc6a1bee",
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::StrategicRegionAlert,
            "strategic_region_alert_entry",
            0x01e5_0ce0,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::StrategicRegion,
            "strategic_region_item",
            0x01e5_0f80,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::ArmyGroupBox,
            "army_group_box",
            0x0187_65d0,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::FleetRowBox,
            "fleet_row_item_box",
            0x01e0_0220,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::ReserveFleetRowBox,
            "reserve_fleet_row_item_box",
            0x01e0_0350,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::FreeAirWings,
            "air_theatre_free_wings_item",
            0x01e4_7dc0,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::AirWing,
            "air_theatre_wing_item",
            0x01e4_8900,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::AirTheatre,
            "air_theatre_item",
            0x01e4_84f0,
            JOB_A,
            SHA_A,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::AirTheatreGroupRow,
            "air_theatre_group_row_item",
            0x01e4_8290,
            JOB_B,
            SHA_B,
        ),
        theatre_claim(
            AuthenticatedTheatreTemplateKind::AirTheatreGroup,
            "air_theatre_group_item",
            0x01e4_8030,
            JOB_B,
            SHA_B,
        ),
    ]
}

fn theatre_claim(
    kind: AuthenticatedTheatreTemplateKind,
    root_token: &'static str,
    controller_rva: u32,
    evidence_job_id: &'static str,
    evidence_artifact_sha256: &'static str,
) -> ExactA729TheatreTemplateClaim {
    ExactA729TheatreTemplateClaim {
        kind,
        root_token,
        controller_rva,
        evidence_job_id,
        evidence_artifact_sha256,
    }
}

fn topbar_claim(
    slot: AuthenticatedHudSemanticSlot,
    lookup_token: &'static str,
    evidence_job_id: &'static str,
    evidence_artifact_sha256: &'static str,
) -> ExactA729ControllerClaim {
    claim(
        slot,
        ControllerPanel::Topbar,
        lookup_token,
        0x0188_ba30,
        evidence_job_id,
        evidence_artifact_sha256,
    )
}

fn claim(
    slot: AuthenticatedHudSemanticSlot,
    panel: ControllerPanel,
    lookup_token: &'static str,
    controller_rva: u32,
    evidence_job_id: &'static str,
    evidence_artifact_sha256: &'static str,
) -> ExactA729ControllerClaim {
    ExactA729ControllerClaim {
        slot,
        panel,
        lookup_token,
        controller_rva,
        evidence_job_id,
        evidence_artifact_sha256,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::ContentLayerKind;
    use std::collections::BTreeSet;

    fn test_content() -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "1".repeat(64),
            normalized_game_relative_path: "interface/test.gui".to_string(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".to_string(),
            source_sha256: "2".repeat(64),
            payload_sha256: "3".repeat(64),
            payload_bytes: 1,
        }
    }

    fn test_action_binding(
        window_id: &str,
        runtime_instance_token: &str,
        frame_generation: u64,
        child_path: Vec<u32>,
        slot: AuthenticatedHudSemanticSlot,
        action: AuthenticatedViewerActionKind,
    ) -> (
        AuthenticatedStaticActionTarget,
        AuthenticatedStaticActionBinding,
    ) {
        let target = AuthenticatedStaticActionTarget {
            window_id: window_id.to_string(),
            runtime_instance_token: runtime_instance_token.to_string(),
            frame_generation,
            child_path,
        };
        let binding = AuthenticatedStaticActionBinding {
            target: target.clone(),
            slot,
            action,
            slot_index_identity_sha256: "4".repeat(64),
            source_content: test_content(),
            authored_tree_identity_sha256: "5".repeat(64),
        };
        (target, binding)
    }

    fn test_slot_index() -> AuthenticatedHudSemanticSlotIndex {
        let mut bindings = BTreeMap::new();
        for (ordinal, claim) in exact_a729_controller_claims().into_iter().enumerate() {
            bindings.insert(
                claim.slot,
                AuthenticatedHudSemanticSlotBinding {
                    slot: claim.slot,
                    index_identity_sha256: "8".repeat(64),
                    source_content: test_content(),
                    surface_id: format!("{:?}", claim.panel),
                    authored_tree_identity_sha256: "9".repeat(64),
                    child_path: vec![u32::try_from(ordinal).expect("test slot count")],
                    engine_lookup_token: claim.lookup_token.to_string(),
                    controller_rva: claim.controller_rva,
                    evidence_job_id: claim.evidence_job_id.to_string(),
                    evidence_artifact_sha256: claim.evidence_artifact_sha256.to_string(),
                },
            );
        }
        AuthenticatedHudSemanticSlotIndex {
            index_identity_sha256: "8".repeat(64),
            exact_binary_sha256: EXACT_A729_BINARY_SHA256.to_string(),
            evidence_bundle_sha256: CONTROLLER_EVIDENCE_BUNDLE_SHA256.to_string(),
            bindings,
            identity_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn exact_claim_set_has_unique_typed_slots_and_valid_receipts() {
        let claims = exact_a729_controller_claims();
        let mut slots = BTreeSet::new();
        for claim in &claims {
            assert!(slots.insert(claim.slot), "duplicate slot {:?}", claim.slot);
            assert!(!claim.lookup_token.is_empty());
            assert!(!claim.evidence_job_id.is_empty());
            assert!(valid_sha256(claim.evidence_artifact_sha256));
        }
        assert_eq!(claims.len(), 51);
    }

    #[test]
    fn generated_speed_slots_are_controller_tokens_not_geometry() {
        let claims = exact_a729_controller_claims();
        for step in 1..=5 {
            let claim = claims
                .iter()
                .find(|claim| claim.slot == AuthenticatedHudSemanticSlot::TopbarSpeedStep(step))
                .expect("speed slot");
            assert_eq!(claim.lookup_token, format!("button_speedstep{step}"));
            assert_eq!(claim.controller_rva, 0x0188_ba30);
        }
    }

    #[test]
    fn strategic_region_and_new_group_controllers_cannot_authenticate_theatre_entry() {
        let claims = exact_a729_controller_claims();
        assert!(claims.iter().all(|claim| {
            claim.controller_rva != 0x01e5_0f80
                && claim.controller_rva != 0x01e5_0a90
                && claim.controller_rva != 0x01e5_9240
        }));
        for forbidden in [
            "background",
            "pinned",
            "naval_access_rule",
            "name",
            "alerts",
        ] {
            assert!(claims.iter().all(|claim| claim.lookup_token != forbidden));
        }
    }

    #[test]
    fn exact_negative_evidence_slots_are_not_revived_as_controller_semantics() {
        let slots = exact_a729_controller_claims()
            .into_iter()
            .map(|claim| format!("{:?}", claim.slot))
            .collect::<BTreeSet<_>>();
        for forbidden in [
            "TheatreEntryPriority1",
            "TheatreEntryPriority2",
            "TheatreEntryPriority3",
            "TheatreEntryBackground",
            "TheatreEntryPinned",
            "TheatreEntryName",
            "MapmodeButton12",
            "TopbarArmyExperienceRatio",
        ] {
            assert!(!slots.contains(forbidden));
        }
    }

    #[test]
    fn persistent_mapmode_twin_set_is_exactly_one_through_eleven() {
        let claims = exact_a729_controller_claims();
        for ordinal in 1..=11 {
            let deselected = claims
                .iter()
                .find(|claim| {
                    claim.slot == AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(ordinal)
                })
                .expect("deselected map-mode twin");
            let selected = claims
                .iter()
                .find(|claim| {
                    claim.slot == AuthenticatedHudSemanticSlot::MapmodeButtonSelected(ordinal)
                })
                .expect("selected map-mode twin");
            assert_eq!(deselected.lookup_token, format!("mapmode_button_{ordinal}"));
            assert_eq!(selected.lookup_token, format!("mapmode_button_{ordinal}_s"));
            assert_eq!(deselected.controller_rva, 0x0156_3c80);
            assert_eq!(selected.controller_rva, 0x0156_3c80);
        }
        assert!(claims
            .iter()
            .all(|claim| claim.lookup_token != "mapmode_button_12"));
        assert!(claims
            .iter()
            .all(|claim| claim.lookup_token != "mapmode_button_12_s"));
    }

    #[test]
    fn product_action_projection_is_typed_and_rejects_out_of_range_slots() {
        for step in 1..=5 {
            assert_eq!(
                static_slot_action(AuthenticatedHudSemanticSlot::TopbarSpeedStep(step)),
                Some(ReplayViewerAction::Transport(
                    TransportViewerAction::SetPlaybackSpeedStep(step)
                ))
            );
        }
        for ordinal in 1..=11 {
            for slot in [
                AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(ordinal),
                AuthenticatedHudSemanticSlot::MapmodeButtonSelected(ordinal),
            ] {
                assert_eq!(
                    static_slot_action(slot),
                    Some(ReplayViewerAction::Mapmode(
                        MapmodeViewerAction::SelectMapmodeButton(ordinal)
                    ))
                );
            }
        }
        assert_eq!(
            static_slot_action(AuthenticatedHudSemanticSlot::TopbarSpeedStep(0)),
            None
        );
        assert_eq!(
            static_slot_action(AuthenticatedHudSemanticSlot::TopbarSpeedStep(6)),
            None
        );
        assert_eq!(
            static_slot_action(AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(12)),
            None
        );
        assert_eq!(
            static_slot_action(AuthenticatedHudSemanticSlot::MapmodeButtonSelected(12)),
            None
        );
        assert_eq!(
            static_slot_action(AuthenticatedHudSemanticSlot::TopbarPlayerFlag),
            None
        );
    }

    #[test]
    fn actual_theatre_template_set_excludes_both_controller_inert_roots() {
        let claims = exact_a729_theatre_template_claims();
        assert_eq!(claims.len(), 17);
        let mut kinds = BTreeSet::new();
        let mut roots = BTreeSet::new();
        for claim in claims {
            assert!(kinds.insert(claim.kind));
            assert!(roots.insert(claim.root_token));
            assert_ne!(claim.controller_rva, 0);
            assert!(!claim.evidence_job_id.is_empty());
            assert!(valid_sha256(claim.evidence_artifact_sha256));
        }
        assert!(!roots.contains("theatre_selection_entry"));
        assert!(!roots.contains("theatre_selection_alert_entry"));
    }

    #[test]
    fn action_census_resolves_speed_and_both_mapmode_twins_only_for_exact_frame_target() {
        let mut actions = BTreeMap::new();
        for (target, binding) in [
            test_action_binding(
                "topbar",
                "topbar#7",
                41,
                vec![1],
                AuthenticatedHudSemanticSlot::TopbarSpeedStep(3),
                AuthenticatedViewerActionKind::SetPlaybackSpeedStep(3),
            ),
            test_action_binding(
                "MapmodesInterface_Ingame",
                "mapmodes#4",
                41,
                vec![2],
                AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(6),
                AuthenticatedViewerActionKind::SelectMapmodeButton(6),
            ),
            test_action_binding(
                "MapmodesInterface_Ingame",
                "mapmodes#4",
                41,
                vec![3],
                AuthenticatedHudSemanticSlot::MapmodeButtonSelected(6),
                AuthenticatedViewerActionKind::SelectMapmodeButton(6),
            ),
        ] {
            insert_static_action_binding(&mut actions, target, binding).expect("unique action");
        }
        let identity_sha256 = static_action_census_digest(&actions);
        let census = AuthenticatedStaticHudActionCensus {
            actions,
            identity_sha256,
        };
        assert_eq!(
            census.resolve("topbar", "topbar#7", 41, &[1]),
            Some(AuthenticatedViewerActionKind::SetPlaybackSpeedStep(3))
        );
        for path in [vec![2], vec![3]] {
            assert_eq!(
                census.resolve("MapmodesInterface_Ingame", "mapmodes#4", 41, &path),
                Some(AuthenticatedViewerActionKind::SelectMapmodeButton(6))
            );
        }
        assert_eq!(census.resolve("topbar", "topbar#7", 40, &[1]), None);
        assert_eq!(census.resolve("topbar", "topbar#8", 41, &[1]), None);
        assert_eq!(census.resolve("other", "topbar#7", 41, &[1]), None);
        assert_eq!(census.resolve("topbar", "topbar#7", 41, &[9]), None);
    }

    #[test]
    fn action_lease_guard_rejects_stale_reopened_cross_window_and_cross_index_inputs() {
        let index = "6".repeat(64);
        assert_eq!(
            validate_static_action_lease(true, "topbar", "topbar", "topbar#1", &index, &index),
            Ok(())
        );
        for rejection in [
            validate_static_action_lease(false, "topbar", "topbar", "topbar#1", &index, &index),
            validate_static_action_lease(true, "topbar", "mapmodes", "topbar#1", &index, &index),
            validate_static_action_lease(true, "topbar", "topbar", " ", &index, &index),
            validate_static_action_lease(
                true,
                "topbar",
                "topbar",
                "topbar#1",
                &index,
                &"7".repeat(64),
            ),
        ] {
            assert_eq!(
                rejection,
                Err(AuthenticatedSemanticProviderRejection::ActionLeaseMismatch)
            );
        }
    }

    #[test]
    fn action_census_rejects_duplicate_or_mismatched_target_insertion() {
        let mut actions = BTreeMap::new();
        let (target, binding) = test_action_binding(
            "topbar",
            "topbar#1",
            2,
            vec![4],
            AuthenticatedHudSemanticSlot::TopbarSpeedStep(1),
            AuthenticatedViewerActionKind::SetPlaybackSpeedStep(1),
        );
        insert_static_action_binding(&mut actions, target.clone(), binding.clone())
            .expect("first action");
        assert_eq!(
            insert_static_action_binding(&mut actions, target.clone(), binding),
            Err(AuthenticatedSemanticProviderRejection::DuplicateActionTarget)
        );

        let (other_target, mut other_binding) = test_action_binding(
            "topbar",
            "topbar#1",
            2,
            vec![5],
            AuthenticatedHudSemanticSlot::TopbarSpeedStep(2),
            AuthenticatedViewerActionKind::SetPlaybackSpeedStep(2),
        );
        other_binding.target.child_path = vec![6];
        assert_eq!(
            insert_static_action_binding(&mut actions, other_target, other_binding),
            Err(AuthenticatedSemanticProviderRejection::DuplicateActionTarget)
        );
    }

    #[test]
    fn semantic_projection_selects_one_mapmode_twin_and_never_projects_xp_ratios() {
        let slots = test_slot_index();
        let mut hud = HudBinding::default();
        hud.mapmode.selected_button = 5;
        hud.mapmode.use_four_button_layout = Some(true);
        hud.topbar.army_experience = Some("12".to_string());
        hud.topbar.navy_experience = Some("34".to_string());
        hud.topbar.air_experience = Some("56".to_string());
        hud.topbar.army_xp_ratio = Some(0.12);
        hud.topbar.navy_xp_ratio = Some(0.34);
        hud.topbar.air_xp_ratio = Some(0.56);
        let projection =
            project_exact_a729_hud_semantics(&slots, &hud, true).expect("semantic projection");
        assert_eq!(projection.values.len(), 51);
        assert_eq!(
            projection.value(AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(5)),
            Some(&AuthenticatedHudSemanticValue::Hidden)
        );
        assert_eq!(
            projection.value(AuthenticatedHudSemanticSlot::MapmodeButtonSelected(5)),
            Some(&AuthenticatedHudSemanticValue::StandardButton)
        );
        assert_eq!(
            projection.value(AuthenticatedHudSemanticSlot::MapmodeButtonDeselected(6)),
            Some(&AuthenticatedHudSemanticValue::StandardButton)
        );
        assert_eq!(
            projection.value(AuthenticatedHudSemanticSlot::MapmodeButtonSelected(6)),
            Some(&AuthenticatedHudSemanticValue::Hidden)
        );
        assert_eq!(
            projection.value(AuthenticatedHudSemanticSlot::TopbarArmyExperience),
            Some(&AuthenticatedHudSemanticValue::Text("12".to_string()))
        );
        assert!(projection.remains_bound_to(&slots));
        let mut replacement = test_slot_index();
        replacement.identity_sha256 = "b".repeat(64);
        assert!(!projection.remains_bound_to(&replacement));
    }

    #[test]
    fn theatre_projection_distinguishes_unavailable_surface_from_measured_empty_surface() {
        let slots = test_slot_index();
        let hud = HudBinding::default();
        let unavailable = project_exact_a729_hud_semantics(&slots, &hud, false)
            .expect("feature-local unavailable theatre projection");
        let measured_empty = project_exact_a729_hud_semantics(&slots, &hud, true)
            .expect("admitted measured-empty theatre projection");
        assert_eq!(
            unavailable.value(AuthenticatedHudSemanticSlot::TheatreElementsList),
            Some(&AuthenticatedHudSemanticValue::Hidden)
        );
        assert_eq!(
            measured_empty.value(AuthenticatedHudSemanticSlot::TheatreElementsList),
            Some(&AuthenticatedHudSemanticValue::Structural)
        );
        assert_ne!(unavailable.identity_sha256, measured_empty.identity_sha256);
    }

    #[test]
    fn semantic_projection_rejects_inert_mapmode_12_invalid_progress_and_missing_slot() {
        let slots = test_slot_index();
        let mut hud = HudBinding::default();
        hud.mapmode.selected_button = 12;
        hud.mapmode.use_four_button_layout = Some(true);
        assert_eq!(
            project_exact_a729_hud_semantics(&slots, &hud, true),
            Err(AuthenticatedSemanticProviderRejection::InvalidMapmodeSelection(12))
        );

        hud.mapmode.selected_button = 0;
        hud.mapmode.use_four_button_layout = None;
        hud.topbar.fuel_ratio = Some(1.5);
        assert_eq!(
            project_exact_a729_hud_semantics(&slots, &hud, true),
            Err(
                AuthenticatedSemanticProviderRejection::InvalidSemanticValue(
                    AuthenticatedHudSemanticSlot::TopbarFuelRatio
                )
            )
        );

        hud.topbar.fuel_ratio = None;
        let mut incomplete = test_slot_index();
        incomplete
            .bindings
            .remove(&AuthenticatedHudSemanticSlot::TopbarDateText);
        assert_eq!(
            project_exact_a729_hud_semantics(&incomplete, &hud, true),
            Err(AuthenticatedSemanticProviderRejection::MissingSealedSlot(
                AuthenticatedHudSemanticSlot::TopbarDateText
            ))
        );
    }
}
