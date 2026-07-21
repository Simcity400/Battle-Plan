//! Exact-a729 post-refresh observer and fail-closed fixture owner for H-A37.
//!
//! The exact20 receipt resolves `CPowerBalanceView`'s terminal updater as
//! `void(this, u8 full_refresh, u8 range_refresh)` at RVA `0x01C4BAA0` and
//! supplies its entry bytes. Evidence mode may install the real post-original
//! detour while production activation remains false. Pointer-bearing owner
//! fields are consumed only by the guard and are never returned or serialized.

use crate::{
    encode_payload, normalize_a729_controller_state, A729ControllerStateObservation, CarrierError,
    normalize_engine_range_indicator_frame, ChangeCadence, DecisionAvailability, DecisionInput,
    NormalizedControllerState, PowerBalanceInput, PowerBalancePresentationFrame, ResolvedTextRefs,
    SelectedPresentStamp, TrendInput,
    A729_CONTROLLER_EXACT20_AUTHORITY_BOUND,
    A729_CONTROLLER_EXACT20_MANIFEST_SHA256, A729_CONTROLLER_OBSERVER_ACTIVATED,
};
use crate::a729_decision_grid::DecisionGridMaterialization;
use std::collections::BTreeSet;

/// Independent authority receipt for the country-politics launcher and
/// power-balance panel lifecycle.  It supplements (and does not replace) the
/// terminal-controller receipt used by [`A729PostRefreshHookSample`].
pub const A729_PANEL_VISIBILITY_AUTHORITY_BOUND: bool = true;
pub const A729_PANEL_VISIBILITY_JOB_ID: &str =
    "20260720T044622Z-w4-a37-panel-open-visibility-authority-ca46b41d";
pub const A729_PANEL_VISIBILITY_SHARD: &str = "shard-01";
pub const A729_PANEL_VISIBILITY_MANIFEST_SHA256: [u8; 32] = [
    0x2c, 0x92, 0x12, 0x16, 0xfb, 0x9b, 0x82, 0x57, 0x16, 0x68, 0x58, 0x52, 0xc0, 0x26, 0xab, 0xa9,
    0x2b, 0x7d, 0xa8, 0x8a, 0x5c, 0x88, 0xfd, 0xb6, 0x25, 0xa5, 0x69, 0xe4, 0x9a, 0xfe, 0xdd, 0x9c,
];
/// A static lifecycle proof is not a live activation receipt.
pub const A729_PANEL_VISIBILITY_OBSERVER_ACTIVATED: bool = false;

pub const BATTLE_PLAN_A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
pub const POST_REFRESH_RVA: u32 = 0x01c4_baa0;
pub const POWER_BALANCE_VIEW_VTABLE_RVA: u32 = 0x02a4_21c0;
pub const COUNTRY_POLITICS_VIEW_VTABLE_RVA: u32 = 0x029c_2a58;
pub const COUNTRY_POLITICS_POWER_BALANCE_CONTROLLER_OFFSET: usize = 0x7a50;
pub const COUNTRY_POLITICS_POWER_BALANCE_LAUNCHER_OFFSET: usize = 0x7ba0;
pub const COUNTRY_POLITICS_REFRESH_RVA: u32 = 0x0157_0510;
pub const COUNTRY_POLITICS_LAUNCHER_CLICK_RVA: u32 = 0x0157_0450;
pub const CONTROLLER_OPEN_OFFSET: usize = 0x30;
pub const CONTROLLER_GUI_ROOT_OFFSET: usize = 0x48;
pub const GUI_CONTAINER_VISIBILITY_FLAGS_OFFSET: usize = 0xa5;
pub const GUI_CONTAINER_VISIBLE_MASK: u8 = 0x08;
pub const VIEW_SELECTED_RECORD_OFFSET: usize = 0x598;
pub const VIEW_SELECTED_RANGE_OFFSET: usize = 0x5a0;
pub const RECORD_LEFT_SIDE_OFFSET: usize = 0x18;
pub const RECORD_RIGHT_SIDE_OFFSET: usize = 0x20;
pub const RECORD_TRENDING_SIDE_OFFSET: usize = 0x28;
pub const RECORD_CURRENT_VALUE_OFFSET: usize = 0x30;
pub const RECORD_SELECTED_RANGE_OFFSET: usize = 0xb0;
pub const GET_CHANGE_RVA: u32 = 0x00e4_1430;
/// Fresh exact-a729 semantic-leaf export used by the raw-to-PBP2 materializer.
/// This receipt covers the final range geometry/frame writer, splitter writer,
/// effective icon resolver, change formatter, modifier-description formatter,
/// exact change sum, and country-politics launcher publisher.
pub const A729_SEMANTIC_LEAF_JOB_ID: &str =
    "20260721T065201Z-wave6-a37-semantic-leaf-producers-85cdcc4c";
pub const A729_SEMANTIC_LEAF_SHARD: &str = "shard-03";
pub const A729_SEMANTIC_LEAF_MANIFEST_SHA256: [u8; 32] = [
    0x6d, 0x53, 0xa5, 0xde, 0xea, 0xb2, 0xdb, 0x5d, 0x6e, 0x28, 0xbc, 0x88, 0xc4, 0xc6, 0xc8, 0xe6,
    0x30, 0x11, 0xa6, 0xf2, 0x36, 0xce, 0xb8, 0xb7, 0xfd, 0xaa, 0xc1, 0x80, 0x92, 0x28, 0x65, 0x77,
];
/// Instruction-aligned receipt bytes through `PUSH R13`.
pub const POST_REFRESH_ENTRY_SENTINEL: [u8; 17] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x88, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
    0x55,
];
pub const COUNTRY_POLITICS_REFRESH_ENTRY_SENTINEL: [u8; 19] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac, 0x24, 0x10,
    0xff, 0xff, 0xff,
];
pub const COUNTRY_POLITICS_LAUNCHER_CLICK_ENTRY_SENTINEL: [u8; 20] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x81, 0xc1,
    0x50, 0x7a, 0x00, 0x00,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729PostRefreshHookSample {
    pub executable_sha256: [u8; 32],
    pub producer_manifest_sha256: [u8; 32],
    pub panel_visibility_manifest_sha256: [u8; 32],
    pub target_rva: u32,
    pub entry_sentinel: [u8; 17],
    pub current_thread_id: u32,
    pub owner_thread_id: u32,
    pub view_ptr: usize,
    pub view_vtable_rva: u32,
    pub selected_record_ptr: usize,
    pub selected_range_ptr: usize,
    pub controller_open_raw: u8,
    pub controller_gui_root_ptr: usize,
    pub controller_gui_visibility_flags: u8,
    pub full_refresh_raw: u8,
    pub range_refresh_raw: u8,
    pub materialization_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    pub controller_state: A729ControllerStateObservation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedA729PostRefreshObservation {
    pub panel_open: bool,
    pub full_refresh: bool,
    pub range_refresh: bool,
    pub controller_state: NormalizedControllerState,
}

/// Exact runtime-text joins retained from A25. The pointer fields exist only
/// to prove that the context came from the controller widget in this callback;
/// only `context_id` is copied into PBP2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729ObservedTextRef {
    pub widget_ptr: usize,
    pub textbox_owner_ptr: usize,
    pub context_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct A729ObservedTrend {
    pub cadence: ChangeCadence,
    pub magnitude: f64,
    pub resolved_text: A729ObservedTextRef,
}

/// Pointer/ordinal-to-portable range identity proof. Exact a729 does not expose
/// a stable token at `range+0x10` (that field is the upper fixed-point bound),
/// so the only admitted route is selected-pointer equality in the record's
/// stored range vector followed by a unique effective-VFS catalog reverse join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729RangeIdentityWitness {
    pub range_vector_ptr: usize,
    pub range_count: u32,
    pub selected_range_ordinal: u32,
    pub selected_range_ptr: usize,
    pub balance_id: String,
    pub range_id: String,
    pub effective_catalog_sha256: String,
}

/// One decision presenter after the exact `0x141c4aa30` publisher returns.
/// The materializer requires a bijection with the finalized grid rather than
/// trusting pool order or replay-sorting the two publisher routes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729ObservedDecision {
    pub presenter_ptr: usize,
    pub definition_ptr: usize,
    pub source_record_ptr: usize,
    pub decision_id: String,
    pub decision_id_token_object_ptr: usize,
    pub availability: DecisionAvailability,
    pub resolved_name: A729ObservedTextRef,
    pub cost: Option<A729ObservedTextRef>,
    pub timer: Option<A729ObservedTextRef>,
    pub target_tag_ref: Option<String>,
}

/// Raw, pointer-bearing semantic snapshot produced at the post-original
/// `0x141c4baa0` boundary. It is deliberately not encodable. Every pointer is
/// consumed by [`materialize_receipted_semantics`] and erased before PBP2.
#[derive(Clone, Debug, PartialEq)]
pub struct A729SemanticBoundaryObservation {
    pub executable_sha256: [u8; 32],
    pub controller_manifest_sha256: [u8; 32],
    pub semantic_leaf_manifest_sha256: [u8; 32],
    pub callback_sequence: u64,
    pub current_thread_id: u32,
    pub view_ptr: usize,
    pub selected_record_ptr: usize,
    pub balance_definition_ptr: usize,
    pub left_side_definition_ptr: usize,
    pub right_side_definition_ptr: usize,
    pub trending_side_definition_ptr: usize,
    pub selected_range_definition_ptr: usize,
    pub materialization_qpc_ticks: u64,
    pub country_tag: String,
    pub balance_id: String,
    pub balance_id_token_object_ptr: usize,
    pub left_side_id: String,
    pub left_side_id_token_object_ptr: usize,
    pub right_side_id: String,
    pub right_side_id_token_object_ptr: usize,
    pub trending_side_id: String,
    pub trending_side_id_token_object_ptr: usize,
    pub active_range_identity: A729RangeIdentityWitness,
    pub effective_left_icon: String,
    pub effective_right_icon: String,
    pub left_icon_widget_ptr: usize,
    pub right_icon_widget_ptr: usize,
    pub launcher_selected_record_ptr: usize,
    pub launcher_visible: bool,
    pub launcher_percentage: A729ObservedTextRef,
    pub launcher_levels: A729ObservedTextRef,
    pub launcher_icon_sprite: String,
    pub launcher_icon_widget_ptr: usize,
    pub launcher_glow_visible: bool,
    pub engine_range_indicator_frame: u32,
    pub range_indicator_widget_ptr: usize,
    pub range_indicator_top_left: [i32; 2],
    pub trend: Option<A729ObservedTrend>,
    pub modifier_description: A729ObservedTextRef,
    pub balance_name: A729ObservedTextRef,
    pub left_side_name: A729ObservedTextRef,
    pub right_side_name: A729ObservedTextRef,
    pub active_range_name: A729ObservedTextRef,
    pub balance_tooltip: A729ObservedTextRef,
    pub active_range_tooltip: A729ObservedTextRef,
    pub decision_grid: DecisionGridMaterialization,
    /// Exact finalized grid child pointers in the same order as
    /// `decision_grid.slots`. These are erased after the presenter bijection.
    pub finalized_decision_presenter_ptrs: Vec<usize>,
    pub decisions: Vec<A729ObservedDecision>,
}

/// Recorder-owned metadata unavailable at the controller callback itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729SemanticFrameEnvelope {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub selected_present: SelectedPresentStamp,
    pub completeness_record_identity: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookAdmissionError {
    Inactive,
    AuthorityUnbound,
    ExecutableMismatch,
    ManifestMismatch,
    PanelVisibilityAuthorityUnbound,
    PanelVisibilityManifestMismatch,
    TargetMismatch,
    SentinelMismatch,
    ThreadMismatch,
    OwnerMismatch,
    InvalidFlags,
    PanelNotOpen,
    LifecycleMismatch,
    InvalidControllerState(CarrierError),
    Materialization(String),
    FrameJoinMismatch,
    Carrier(CarrierError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729LauncherVisibilityRefreshSample {
    pub executable_sha256: [u8; 32],
    pub producer_manifest_sha256: [u8; 32],
    pub target_rva: u32,
    pub entry_sentinel: [u8; 19],
    pub current_thread_id: u32,
    pub owner_thread_id: u32,
    pub country_politics_view_ptr: usize,
    pub country_politics_view_vtable_rva: u32,
    /// Result of the exact outer `CCountryPoliticsView` active predicate.  The
    /// refresh function does not write launcher state when this is false.
    pub country_politics_active_raw: u8,
    pub launcher_root_ptr: usize,
    pub launcher_visibility_flags: u8,
    pub selected_record_ptr: usize,
    pub controller_ptr: usize,
    pub controller_vtable_rva: u32,
    pub controller_open_raw: u8,
    pub controller_gui_root_ptr: usize,
    pub controller_gui_visibility_flags: u8,
    pub controller_selected_record_ptr: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729PanelToggleHookSample {
    pub executable_sha256: [u8; 32],
    pub producer_manifest_sha256: [u8; 32],
    pub target_rva: u32,
    pub entry_sentinel: [u8; 20],
    pub current_thread_id: u32,
    pub owner_thread_id: u32,
    pub country_politics_view_ptr: usize,
    pub country_politics_view_vtable_rva: u32,
    pub controller_ptr: usize,
    pub controller_vtable_rva: u32,
    pub launcher_visible_before_raw: u8,
    pub panel_open_before_raw: u8,
    pub panel_open_after_raw: u8,
    pub controller_gui_root_ptr: usize,
    pub controller_gui_visibility_flags_after: u8,
    pub selected_record_ptr_after: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedA729PanelVisibilityObservation {
    pub launcher_visible: bool,
    pub panel_open: bool,
    pub selected_balance_present: bool,
}

/// Authenticate a captured callback sample against the exact20 contract.
/// This pure guard is testable while installation remains disabled.
pub fn validate_receipted_sample(
    sample: &A729PostRefreshHookSample,
) -> Result<AuthenticatedA729PostRefreshObservation, HookAdmissionError> {
    if !A729_CONTROLLER_EXACT20_AUTHORITY_BOUND {
        return Err(HookAdmissionError::AuthorityUnbound);
    }
    if sample.executable_sha256 != BATTLE_PLAN_A729_EXECUTABLE_SHA256 {
        return Err(HookAdmissionError::ExecutableMismatch);
    }
    if sample.producer_manifest_sha256 != A729_CONTROLLER_EXACT20_MANIFEST_SHA256 {
        return Err(HookAdmissionError::ManifestMismatch);
    }
    if !A729_PANEL_VISIBILITY_AUTHORITY_BOUND {
        return Err(HookAdmissionError::PanelVisibilityAuthorityUnbound);
    }
    if sample.panel_visibility_manifest_sha256 != A729_PANEL_VISIBILITY_MANIFEST_SHA256 {
        return Err(HookAdmissionError::PanelVisibilityManifestMismatch);
    }
    if sample.target_rva != POST_REFRESH_RVA {
        return Err(HookAdmissionError::TargetMismatch);
    }
    if sample.entry_sentinel != POST_REFRESH_ENTRY_SENTINEL {
        return Err(HookAdmissionError::SentinelMismatch);
    }
    if sample.current_thread_id == 0
        || sample.current_thread_id != sample.owner_thread_id
    {
        return Err(HookAdmissionError::ThreadMismatch);
    }
    if sample.view_ptr == 0
        || sample.view_vtable_rva != POWER_BALANCE_VIEW_VTABLE_RVA
        || sample.selected_record_ptr == 0
        || sample.selected_range_ptr == 0
        || sample.controller_gui_root_ptr == 0
    {
        return Err(HookAdmissionError::OwnerMismatch);
    }
    if sample.full_refresh_raw > 1
        || sample.range_refresh_raw > 1
        || sample.controller_open_raw > 1
        || sample.materialization_qpc_ticks == 0
        || sample.qpc_frequency_hz == 0
    {
        return Err(HookAdmissionError::InvalidFlags);
    }
    if sample.controller_open_raw == 0
        || sample.controller_gui_visibility_flags & GUI_CONTAINER_VISIBLE_MASK == 0
    {
        return Err(HookAdmissionError::PanelNotOpen);
    }
    let controller_state = normalize_a729_controller_state(&sample.controller_state)
        .map_err(HookAdmissionError::InvalidControllerState)?;
    Ok(AuthenticatedA729PostRefreshObservation {
        panel_open: true,
        full_refresh: sample.full_refresh_raw != 0,
        range_refresh: sample.range_refresh_raw != 0,
        controller_state,
    })
}

fn semantic_error(message: impl Into<String>) -> HookAdmissionError {
    HookAdmissionError::Materialization(message.into())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value.bytes().any(|byte| byte != b'0')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_observed_text_ref(
    value: &A729ObservedTextRef,
    label: &str,
) -> Result<String, HookAdmissionError> {
    if value.widget_ptr == 0
        || value.textbox_owner_ptr == 0
        || value.context_id.trim().is_empty()
        || value.context_id.chars().any(char::is_control)
    {
        return Err(semantic_error(format!(
            "A37 {label} lacks an exact A25 owner/context join"
        )));
    }
    Ok(value.context_id.clone())
}

/// Convert one exact pointer-bearing callback observation into the sole
/// pointer-free PBP2 frame. This is the semantic materializer: it consumes the
/// controller/leaf receipts, proves every cross-object join, validates the
/// finalized decision-grid bijection, and only then erases process addresses.
///
/// A pre-authored [`PowerBalancePresentationFrame`] is intentionally not an
/// input. Missing range identity, A25 text, icon, decision, or Present custody
/// returns an error; there is no save/game-file/default fallback.
pub fn materialize_receipted_semantics(
    sample: &A729PostRefreshHookSample,
    semantic: &A729SemanticBoundaryObservation,
    envelope: &A729SemanticFrameEnvelope,
) -> Result<PowerBalancePresentationFrame, HookAdmissionError> {
    let authenticated = validate_receipted_sample(sample)?;
    if semantic.executable_sha256 != BATTLE_PLAN_A729_EXECUTABLE_SHA256
        || semantic.controller_manifest_sha256 != A729_CONTROLLER_EXACT20_MANIFEST_SHA256
        || semantic.semantic_leaf_manifest_sha256 != A729_SEMANTIC_LEAF_MANIFEST_SHA256
    {
        return Err(HookAdmissionError::ManifestMismatch);
    }
    if semantic.callback_sequence == 0
        || semantic.current_thread_id != sample.current_thread_id
        || semantic.view_ptr != sample.view_ptr
        || semantic.selected_record_ptr != sample.selected_record_ptr
        || semantic.selected_range_definition_ptr != sample.selected_range_ptr
        || semantic.materialization_qpc_ticks != sample.materialization_qpc_ticks
        || semantic.balance_definition_ptr == 0
        || semantic.left_side_definition_ptr == 0
        || semantic.right_side_definition_ptr == 0
        || semantic.trending_side_definition_ptr == 0
        || semantic.left_side_definition_ptr == semantic.right_side_definition_ptr
        || semantic.balance_id_token_object_ptr
            != semantic.balance_definition_ptr.checked_add(0x10).unwrap_or(0)
        || semantic.left_side_id_token_object_ptr
            != semantic.left_side_definition_ptr.checked_add(0x10).unwrap_or(0)
        || semantic.right_side_id_token_object_ptr
            != semantic.right_side_definition_ptr.checked_add(0x10).unwrap_or(0)
        || semantic.trending_side_id_token_object_ptr
            != semantic
                .trending_side_definition_ptr
                .checked_add(0x10)
                .unwrap_or(0)
    {
        return Err(HookAdmissionError::FrameJoinMismatch);
    }
    if semantic.left_side_id != sample.controller_state.left_side_id
        || semantic.right_side_id != sample.controller_state.right_side_id
        || semantic.trending_side_id != sample.controller_state.trending_side_id
        || semantic.engine_range_indicator_frame
            != sample.controller_state.engine_range_indicator_frame
    {
        return Err(HookAdmissionError::FrameJoinMismatch);
    }
    let expected_active_definition = if sample.controller_state.current_value_raw < 0 {
        semantic.left_side_definition_ptr
    } else {
        semantic.right_side_definition_ptr
    };
    let observed_active_id = if expected_active_definition == semantic.left_side_definition_ptr {
        semantic.left_side_id.as_str()
    } else {
        semantic.right_side_id.as_str()
    };
    if observed_active_id != authenticated.controller_state.active_side_id
        || (semantic.trending_side_definition_ptr != semantic.left_side_definition_ptr
            && semantic.trending_side_definition_ptr != semantic.right_side_definition_ptr)
        || semantic.left_icon_widget_ptr == 0
        || semantic.right_icon_widget_ptr == 0
        || semantic.left_icon_widget_ptr == semantic.right_icon_widget_ptr
        || semantic.range_indicator_widget_ptr == 0
        || semantic.effective_left_icon.trim().is_empty()
        || semantic.effective_right_icon.trim().is_empty()
        || semantic.balance_id.trim().is_empty()
    {
        return Err(HookAdmissionError::FrameJoinMismatch);
    }
    let range_identity = &semantic.active_range_identity;
    if range_identity.range_vector_ptr == 0
        || range_identity.range_count == 0
        || range_identity.selected_range_ordinal >= range_identity.range_count
        || range_identity.selected_range_ptr != semantic.selected_range_definition_ptr
        || range_identity.balance_id != semantic.balance_id
        || range_identity.range_id.trim().is_empty()
        || !is_lower_sha256(&range_identity.effective_catalog_sha256)
    {
        return Err(semantic_error(
            "A37 active range lacks the exact ordinal/effective-catalog identity join",
        ));
    }
    if semantic.launcher_selected_record_ptr != semantic.selected_record_ptr
        || !semantic.launcher_visible
        || semantic.launcher_icon_widget_ptr == 0
        || semantic.launcher_icon_sprite.trim().is_empty()
        || semantic.launcher_icon_sprite
            != if sample.controller_state.current_value_raw < 0 {
                semantic.effective_left_icon.as_str()
            } else {
                semantic.effective_right_icon.as_str()
            }
    {
        return Err(HookAdmissionError::FrameJoinMismatch);
    }
    if envelope.status_flags != 0
        || envelope.frame_hour != envelope.selected_present.frame_hour
        || envelope.selected_present.qpc_frequency_hz != sample.qpc_frequency_hz
        || semantic.materialization_qpc_ticks
            > envelope.selected_present.selected_present_qpc_ticks
        || envelope.completeness_record_identity.trim().is_empty()
    {
        return Err(HookAdmissionError::FrameJoinMismatch);
    }

    let range_indicator_frame =
        normalize_engine_range_indicator_frame(semantic.engine_range_indicator_frame)
            .map_err(HookAdmissionError::Carrier)?;
    if range_indicator_frame != authenticated.controller_state.range_indicator_frame {
        return Err(HookAdmissionError::FrameJoinMismatch);
    }

    let trend = match (&semantic.trend, sample.controller_state.change_raw.cmp(&0)) {
        (None, std::cmp::Ordering::Equal) => None,
        (Some(observed), std::cmp::Ordering::Less | std::cmp::Ordering::Greater)
            if observed.magnitude.is_finite() && observed.magnitude > 0.0 =>
        {
            Some(TrendInput {
                cadence: observed.cadence,
                magnitude: observed.magnitude,
                resolved_text_ref: validate_observed_text_ref(
                    &observed.resolved_text,
                    "change text",
                )?,
            })
        }
        _ => {
            return Err(semantic_error(
                "A37 resolved trend does not match the exact signed change",
            ))
        }
    };

    if semantic.finalized_decision_presenter_ptrs.len() != semantic.decision_grid.slots.len()
        || semantic.decisions.len() != semantic.decision_grid.slots.len()
        || semantic.decision_grid.ordered_decision_ids.len()
            != semantic.decision_grid.slots.len()
    {
        return Err(semantic_error(
            "A37 finalized decision grid/presenter/semantic counts differ",
        ));
    }
    let mut presenter_ptrs = BTreeSet::new();
    let mut definition_ptrs = BTreeSet::new();
    let mut decision_ids = BTreeSet::new();
    let mut decisions = Vec::with_capacity(semantic.decisions.len());
    for (((presenter_ptr, slot), ordered_id), observed) in semantic
        .finalized_decision_presenter_ptrs
        .iter()
        .zip(&semantic.decision_grid.slots)
        .zip(&semantic.decision_grid.ordered_decision_ids)
        .zip(&semantic.decisions)
    {
        if *presenter_ptr == 0
            || *presenter_ptr != observed.presenter_ptr
            || observed.definition_ptr == 0
            || observed.source_record_ptr == 0
            || observed.decision_id_token_object_ptr
                != observed.definition_ptr.checked_add(0x120).unwrap_or(0)
            || slot.bound_decision_id != *ordered_id
            || observed.decision_id != *ordered_id
            || !presenter_ptrs.insert(*presenter_ptr)
            || !definition_ptrs.insert(observed.definition_ptr)
            || !decision_ids.insert(observed.decision_id.as_str())
        {
            return Err(semantic_error(
                "A37 decision presenter/definition/grid bijection failed",
            ));
        }
        decisions.push(DecisionInput {
            decision_id: observed.decision_id.clone(),
            source_record_id: format!(
                "a729:{}:{}",
                semantic.balance_id, slot.slot_id
            ),
            availability: observed.availability.clone(),
            resolved_name_text_ref: validate_observed_text_ref(
                &observed.resolved_name,
                "decision name",
            )?,
            cost_text_ref: observed
                .cost
                .as_ref()
                .map(|value| validate_observed_text_ref(value, "decision cost"))
                .transpose()?,
            timer_text_ref: observed
                .timer
                .as_ref()
                .map(|value| validate_observed_text_ref(value, "decision timer"))
                .transpose()?,
            target_tag_ref: observed.target_tag_ref.clone(),
        });
    }

    let balance = PowerBalanceInput {
        source_record_id: format!(
            "a729:{}:{}:selected-ordinal-0",
            semantic.country_tag, semantic.balance_id
        ),
        country_tag: semantic.country_tag.clone(),
        balance_id: semantic.balance_id.clone(),
        display_ordinal: 0,
        active_side_id: authenticated.controller_state.active_side_id,
        trending_side_id: semantic.trending_side_id.clone(),
        active_range_id: range_identity.range_id.clone(),
        effective_left_icon: semantic.effective_left_icon.clone(),
        effective_right_icon: semantic.effective_right_icon.clone(),
        launcher_visible: true,
        launcher_percentage_text_ref: Some(validate_observed_text_ref(
            &semantic.launcher_percentage,
            "launcher percentage",
        )?),
        launcher_levels_text_ref: Some(validate_observed_text_ref(
            &semantic.launcher_levels,
            "launcher levels",
        )?),
        launcher_icon_sprite: Some(semantic.launcher_icon_sprite.clone()),
        launcher_glow_visible: semantic.launcher_glow_visible,
        panel_open: true,
        bar_state: authenticated.controller_state.bar_state,
        range_indicator_frame,
        range_indicator_top_left: semantic.range_indicator_top_left,
        trend,
        power_balance_modifier_desc_text_ref: validate_observed_text_ref(
            &semantic.modifier_description,
            "modifier description",
        )?,
        decisions_complete: true,
        decisions,
        resolved_text_refs: ResolvedTextRefs {
            balance_name: validate_observed_text_ref(&semantic.balance_name, "balance name")?,
            left_side_name: validate_observed_text_ref(
                &semantic.left_side_name,
                "left side name",
            )?,
            right_side_name: validate_observed_text_ref(
                &semantic.right_side_name,
                "right side name",
            )?,
            active_range_name: validate_observed_text_ref(
                &semantic.active_range_name,
                "active range name",
            )?,
            balance_tooltip: validate_observed_text_ref(
                &semantic.balance_tooltip,
                "balance tooltip",
            )?,
            active_range_tooltip: validate_observed_text_ref(
                &semantic.active_range_tooltip,
                "active range tooltip",
            )?,
        },
    };
    let frame = PowerBalancePresentationFrame {
        elapsed_wall_ms: envelope.elapsed_wall_ms,
        status_flags: 0,
        frame_hour: envelope.frame_hour,
        materialization_qpc_ticks: semantic.materialization_qpc_ticks,
        selected_present: envelope.selected_present.clone(),
        producer_manifest_sha256: A729_CONTROLLER_EXACT20_MANIFEST_SHA256,
        completeness_record_identity: envelope.completeness_record_identity.clone(),
        expected_balance_count: 1,
        balances: vec![balance],
    };
    crate::validate_frame(&frame).map_err(HookAdmissionError::Carrier)?;
    Ok(frame)
}

fn validate_panel_authority(
    executable_sha256: [u8; 32],
    producer_manifest_sha256: [u8; 32],
    current_thread_id: u32,
    owner_thread_id: u32,
) -> Result<(), HookAdmissionError> {
    if !A729_PANEL_VISIBILITY_AUTHORITY_BOUND {
        return Err(HookAdmissionError::PanelVisibilityAuthorityUnbound);
    }
    if executable_sha256 != BATTLE_PLAN_A729_EXECUTABLE_SHA256 {
        return Err(HookAdmissionError::ExecutableMismatch);
    }
    if producer_manifest_sha256 != A729_PANEL_VISIBILITY_MANIFEST_SHA256 {
        return Err(HookAdmissionError::PanelVisibilityManifestMismatch);
    }
    if current_thread_id == 0 || current_thread_id != owner_thread_id {
        return Err(HookAdmissionError::ThreadMismatch);
    }
    Ok(())
}

/// Validate the country-politics refresh after its original body returns.
/// `launcher_visible` is the actual container bit written by `0x141570510`,
/// not an inference from DLC ownership or the authored GUI file.
pub fn validate_launcher_visibility_refresh(
    sample: &A729LauncherVisibilityRefreshSample,
) -> Result<AuthenticatedA729PanelVisibilityObservation, HookAdmissionError> {
    validate_panel_authority(
        sample.executable_sha256,
        sample.producer_manifest_sha256,
        sample.current_thread_id,
        sample.owner_thread_id,
    )?;
    if sample.target_rva != COUNTRY_POLITICS_REFRESH_RVA {
        return Err(HookAdmissionError::TargetMismatch);
    }
    if sample.entry_sentinel != COUNTRY_POLITICS_REFRESH_ENTRY_SENTINEL {
        return Err(HookAdmissionError::SentinelMismatch);
    }
    if sample.country_politics_view_ptr == 0
        || sample.country_politics_view_vtable_rva != COUNTRY_POLITICS_VIEW_VTABLE_RVA
        || sample.launcher_root_ptr == 0
        || sample.controller_ptr == 0
        || sample.controller_vtable_rva != POWER_BALANCE_VIEW_VTABLE_RVA
        || sample.controller_gui_root_ptr == 0
    {
        return Err(HookAdmissionError::OwnerMismatch);
    }
    if sample.country_politics_active_raw > 1 || sample.controller_open_raw > 1 {
        return Err(HookAdmissionError::InvalidFlags);
    }
    if sample.country_politics_active_raw == 0 {
        return Err(HookAdmissionError::LifecycleMismatch);
    }
    let launcher_visible =
        sample.launcher_visibility_flags & GUI_CONTAINER_VISIBLE_MASK != 0;
    let selected_balance_present = sample.selected_record_ptr != 0;
    let panel_open = sample.controller_open_raw != 0;
    let panel_gui_visible =
        sample.controller_gui_visibility_flags & GUI_CONTAINER_VISIBLE_MASK != 0;
    if launcher_visible != selected_balance_present
        || panel_open != panel_gui_visible
        || (panel_open
            && (!selected_balance_present
                || sample.controller_selected_record_ptr != sample.selected_record_ptr))
        || (!launcher_visible && panel_open)
    {
        return Err(HookAdmissionError::LifecycleMismatch);
    }
    Ok(AuthenticatedA729PanelVisibilityObservation {
        launcher_visible,
        panel_open,
        selected_balance_present,
    })
}

/// Validate the post-original state of the launcher click callback.  The
/// exact callback closes an open panel, or closes competing politics panels
/// and opens this one when the launcher has a selected balance.
pub fn validate_panel_toggle(
    sample: &A729PanelToggleHookSample,
) -> Result<AuthenticatedA729PanelVisibilityObservation, HookAdmissionError> {
    validate_panel_authority(
        sample.executable_sha256,
        sample.producer_manifest_sha256,
        sample.current_thread_id,
        sample.owner_thread_id,
    )?;
    if sample.target_rva != COUNTRY_POLITICS_LAUNCHER_CLICK_RVA {
        return Err(HookAdmissionError::TargetMismatch);
    }
    if sample.entry_sentinel != COUNTRY_POLITICS_LAUNCHER_CLICK_ENTRY_SENTINEL {
        return Err(HookAdmissionError::SentinelMismatch);
    }
    if sample.country_politics_view_ptr == 0
        || sample.country_politics_view_vtable_rva != COUNTRY_POLITICS_VIEW_VTABLE_RVA
        || sample.controller_ptr == 0
        || sample.controller_vtable_rva != POWER_BALANCE_VIEW_VTABLE_RVA
        || sample.controller_gui_root_ptr == 0
    {
        return Err(HookAdmissionError::OwnerMismatch);
    }
    if sample.launcher_visible_before_raw > 1
        || sample.panel_open_before_raw > 1
        || sample.panel_open_after_raw > 1
    {
        return Err(HookAdmissionError::InvalidFlags);
    }
    let launcher_visible = sample.launcher_visible_before_raw != 0;
    let panel_open_before = sample.panel_open_before_raw != 0;
    let panel_open = sample.panel_open_after_raw != 0;
    let panel_gui_visible =
        sample.controller_gui_visibility_flags_after & GUI_CONTAINER_VISIBLE_MASK != 0;
    if !launcher_visible
        || panel_open == panel_open_before
        || panel_open != panel_gui_visible
        || (panel_open && sample.selected_record_ptr_after == 0)
    {
        return Err(HookAdmissionError::LifecycleMismatch);
    }
    Ok(AuthenticatedA729PanelVisibilityObservation {
        launcher_visible,
        panel_open,
        selected_balance_present: sample.selected_record_ptr_after != 0,
    })
}

pub fn admit_live_launcher_visibility_refresh(
    sample: &A729LauncherVisibilityRefreshSample,
) -> Result<AuthenticatedA729PanelVisibilityObservation, HookAdmissionError> {
    if !A729_PANEL_VISIBILITY_OBSERVER_ACTIVATED {
        return Err(HookAdmissionError::Inactive);
    }
    validate_launcher_visibility_refresh(sample)
}

pub fn admit_live_panel_toggle(
    sample: &A729PanelToggleHookSample,
) -> Result<AuthenticatedA729PanelVisibilityObservation, HookAdmissionError> {
    if !A729_PANEL_VISIBILITY_OBSERVER_ACTIVATED {
        return Err(HookAdmissionError::Inactive);
    }
    validate_panel_toggle(sample)
}

/// Live callback admission remains inert until the same-transition fixture and
/// replay parity gate authorize activation. A caller cannot bypass that gate
/// merely by presenting a structurally valid sample.
pub fn admit_live_sample(
    sample: &A729PostRefreshHookSample,
) -> Result<AuthenticatedA729PostRefreshObservation, HookAdmissionError> {
    if !A729_CONTROLLER_OBSERVER_ACTIVATED {
        return Err(HookAdmissionError::Inactive);
    }
    validate_receipted_sample(sample)
}

/// Encode only the raw exact-epoch semantic observation. Live admission stays
/// inert until the fixture latch is promoted; there is no callback capable of
/// manufacturing a caller-authored final frame.
pub fn encode_live_post_refresh(
    sample: &A729PostRefreshHookSample,
    semantic: &A729SemanticBoundaryObservation,
    envelope: &A729SemanticFrameEnvelope,
) -> Result<Vec<u8>, HookAdmissionError> {
    after_live_admission(sample, || {
        let frame = materialize_receipted_semantics(sample, semantic, envelope)?;
        encode_payload(&frame).map_err(HookAdmissionError::Carrier)
    })
}

fn after_live_admission<T>(
    sample: &A729PostRefreshHookSample,
    materialize: impl FnOnce() -> Result<T, HookAdmissionError>,
) -> Result<T, HookAdmissionError> {
    admit_live_sample(sample)?;
    materialize()
}

// The production/evidence owner is Windows-only. Keeping the portable guards
// above platform-neutral allows offline replay validation without linking a
// process hook library.
#[cfg(windows)]
mod installed_owner {
    use super::*;
    use crate::{validate_frame, ExactPowerBalancePresentationSource};
    use core::ffi::c_void;
    use minhook::{MH_STATUS, MinHook};
    use std::collections::BTreeMap;
    use std::mem;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Performance::{
        QueryPerformanceCounter, QueryPerformanceFrequency,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

    type PostRefreshFn = unsafe extern "system" fn(usize, u8, u8);

    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
    static ORIGINAL_POST_REFRESH: AtomicUsize = AtomicUsize::new(0);
    static CALLBACKS_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
    static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct PostRefreshBoundaryReceipt {
        pub sequence: u64,
        pub current_thread_id: u32,
        pub view_ptr: usize,
        pub view_vtable_rva: u32,
        pub selected_record_ptr: usize,
        pub selected_range_ptr: usize,
        pub controller_open_raw: u8,
        pub controller_gui_root_ptr: usize,
        pub controller_gui_visibility_flags: u8,
        pub full_refresh_raw: u8,
        pub range_refresh_raw: u8,
        pub materialization_qpc_ticks: u64,
        pub qpc_frequency_hz: u64,
        pub current_value_raw: i64,
        pub change_raw: i64,
    }

    /// Pointer-free evidence emitted by the owner fixture. This deliberately
    /// proves only the terminal controller spine. It cannot authenticate side
    /// identities, range identity/frame, text, decisions, icons, or geometry
    /// and therefore cannot mint PBP2 or activate d74.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RawControllerEvidence {
        pub callback_sequence: u64,
        pub panel_open: bool,
        pub gui_visible: bool,
        pub selected_record_present: bool,
        pub selected_range_present: bool,
        pub full_refresh: bool,
        pub range_refresh: bool,
        pub materialization_qpc_ticks: u64,
        pub qpc_frequency_hz: u64,
        pub current_value_raw: i64,
        pub change_raw: i64,
    }

    #[derive(Default)]
    struct HookState {
        latest_boundary: Option<PostRefreshBoundaryReceipt>,
        sealed_frames: BTreeMap<u64, PowerBalancePresentationFrame>,
        rejected_boundaries: u64,
        rejected_frames: u64,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct HookDiagnostics {
        pub installed: bool,
        pub boundary_sequence: u64,
        pub sealed_frame_count: u64,
        pub rejected_boundaries: u64,
        pub rejected_frames: u64,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct InstalledHookAddress {
        pub post_refresh: usize,
    }

    pub struct A729PowerBalanceHookSource;

    impl ExactPowerBalancePresentationSource for A729PowerBalanceHookSource {
        fn is_exact_a729_power_balance_source(&self) -> bool {
            INSTALLED.load(Ordering::Acquire)
                && A729_CONTROLLER_OBSERVER_ACTIVATED
                && A729_PANEL_VISIBILITY_OBSERVER_ACTIVATED
        }

        fn producer_manifest_sha256(&self) -> [u8; 32] {
            A729_CONTROLLER_EXACT20_MANIFEST_SHA256
        }

        fn materialization_qpc_ticks(&self, frame_hour: u64) -> Result<u64, String> {
            Ok(self.frame(frame_hour)?.materialization_qpc_ticks)
        }

        fn selected_present(&self, frame_hour: u64) -> Result<SelectedPresentStamp, String> {
            Ok(self.frame(frame_hour)?.selected_present)
        }

        fn completeness_record_identity(&self, frame_hour: u64) -> Result<String, String> {
            Ok(self.frame(frame_hour)?.completeness_record_identity)
        }

        fn expected_balance_count(&self, frame_hour: u64) -> Result<u32, String> {
            Ok(self.frame(frame_hour)?.expected_balance_count)
        }

        fn collect_frame(&self, frame_hour: u64) -> Result<Vec<crate::PowerBalanceInput>, String> {
            Ok(self.frame(frame_hour)?.balances)
        }
    }

    impl A729PowerBalanceHookSource {
        fn frame(&self, frame_hour: u64) -> Result<PowerBalancePresentationFrame, String> {
            let state = state().lock().map_err(|_| "A37 hook state mutex poisoned")?;
            state
                .sealed_frames
                .get(&frame_hour)
                .cloned()
                .ok_or_else(|| format!("A37 frame {frame_hour} is not sealed"))
        }
    }

    fn state() -> &'static Mutex<HookState> {
        STATE.get_or_init(|| Mutex::new(HookState::default()))
    }

    fn qpc() -> Option<u64> {
        let mut value = 0i64;
        (unsafe { QueryPerformanceCounter(&mut value) } != 0 && value > 0)
            .then_some(value as u64)
    }

    fn frequency() -> Option<u64> {
        let mut value = 0i64;
        (unsafe { QueryPerformanceFrequency(&mut value) } != 0 && value > 0)
            .then_some(value as u64)
    }

    fn rpm(address: usize, length: usize) -> Option<Vec<u8>> {
        if address == 0 || length == 0 {
            return None;
        }
        let mut bytes = vec![0u8; length];
        let mut read = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                address as *const c_void,
                bytes.as_mut_ptr().cast(),
                length,
                &mut read,
            )
        };
        (ok != 0 && read == length).then_some(bytes)
    }

    fn read_usize(address: usize) -> Option<usize> {
        let bytes = rpm(address, mem::size_of::<usize>())?;
        Some(usize::from_le_bytes(bytes.try_into().ok()?))
    }

    fn read_u8(address: usize) -> Option<u8> {
        rpm(address, 1).and_then(|bytes| bytes.first().copied())
    }

    fn read_i64(address: usize) -> Option<i64> {
        let bytes = rpm(address, mem::size_of::<i64>())?;
        Some(i64::from_le_bytes(bytes.try_into().ok()?))
    }

    pub fn latest_raw_controller_evidence() -> Result<RawControllerEvidence, String> {
        if !INSTALLED.load(Ordering::Acquire) {
            return Err("A37 hook is not installed".into());
        }
        let boundary = state()
            .lock()
            .map_err(|_| "A37 hook state mutex poisoned".to_string())?
            .latest_boundary
            .clone()
            .ok_or("A37 has no completed post-refresh callback")?;
        if boundary.current_thread_id == 0
            || boundary.view_ptr == 0
            || boundary.view_vtable_rva != POWER_BALANCE_VIEW_VTABLE_RVA
            || boundary.controller_gui_root_ptr == 0
            || boundary.controller_open_raw > 1
            || boundary.full_refresh_raw > 1
            || boundary.range_refresh_raw > 1
            || boundary.materialization_qpc_ticks == 0
            || boundary.qpc_frequency_hz == 0
        {
            return Err("A37 latest raw callback failed owner/flag/QPC admission".into());
        }
        let gui_visible =
            boundary.controller_gui_visibility_flags & GUI_CONTAINER_VISIBLE_MASK != 0;
        let panel_open = boundary.controller_open_raw != 0;
        if panel_open != gui_visible {
            return Err("A37 controller open byte differs from GUI visibility".into());
        }
        Ok(RawControllerEvidence {
            callback_sequence: boundary.sequence,
            panel_open,
            gui_visible,
            selected_record_present: boundary.selected_record_ptr != 0,
            selected_range_present: boundary.selected_range_ptr != 0,
            full_refresh: boundary.full_refresh_raw != 0,
            range_refresh: boundary.range_refresh_raw != 0,
            materialization_qpc_ticks: boundary.materialization_qpc_ticks,
            qpc_frequency_hz: boundary.qpc_frequency_hz,
            current_value_raw: boundary.current_value_raw,
            change_raw: boundary.change_raw,
        })
    }

    unsafe extern "system" fn hook_post_refresh(view: usize, full: u8, range: u8) {
        CALLBACKS_IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        let original = ORIGINAL_POST_REFRESH.load(Ordering::Acquire);
        if original != 0 {
            let call: PostRefreshFn = unsafe { mem::transmute(original) };
            unsafe { call(view, full, range) };
        }
        let base = MODULE_BASE.load(Ordering::Acquire);
        let receipt = (|| {
            let qpc = qpc()?;
            let qpc_frequency_hz = frequency()?;
            let vtable = read_usize(view)?;
            let view_vtable_rva = u32::try_from(vtable.checked_sub(base)?).ok()?;
            let controller_gui_root_ptr = read_usize(view.checked_add(CONTROLLER_GUI_ROOT_OFFSET)?)?;
            let selected_record_ptr = read_usize(view.checked_add(VIEW_SELECTED_RECORD_OFFSET)?)?;
            let current_value_raw = read_i64(
                selected_record_ptr.checked_add(RECORD_CURRENT_VALUE_OFFSET)?,
            )?;
            type GetChangeFn = unsafe extern "system" fn(usize) -> i64;
            let get_change: GetChangeFn = unsafe {
                mem::transmute(base.checked_add(GET_CHANGE_RVA as usize)?)
            };
            let change_raw = unsafe { get_change(selected_record_ptr) };
            Some(PostRefreshBoundaryReceipt {
                sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
                current_thread_id: unsafe { GetCurrentThreadId() },
                view_ptr: view,
                view_vtable_rva,
                selected_record_ptr,
                selected_range_ptr: read_usize(view.checked_add(VIEW_SELECTED_RANGE_OFFSET)?)?,
                controller_open_raw: read_u8(view.checked_add(CONTROLLER_OPEN_OFFSET)?)?,
                controller_gui_root_ptr,
                controller_gui_visibility_flags: read_u8(
                    controller_gui_root_ptr.checked_add(GUI_CONTAINER_VISIBILITY_FLAGS_OFFSET)?,
                )?,
                full_refresh_raw: full,
                range_refresh_raw: range,
                materialization_qpc_ticks: qpc,
                qpc_frequency_hz,
                current_value_raw,
                change_raw,
            })
        })();
        if let Ok(mut state) = state().lock() {
            match receipt {
                Some(receipt) => state.latest_boundary = Some(receipt),
                None => state.rejected_boundaries = state.rejected_boundaries.saturating_add(1),
            }
        }
        CALLBACKS_IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }

    fn verify_sentinel(target: usize) -> Result<(), String> {
        let actual = rpm(target, POST_REFRESH_ENTRY_SENTINEL.len())
            .ok_or("A37 post-refresh sentinel is unreadable")?;
        if actual != POST_REFRESH_ENTRY_SENTINEL {
            return Err("A37 post-refresh sentinel mismatch".into());
        }
        Ok(())
    }

    /// Install the actual post-original terminal-writer detour. With activation
    /// false this is allowed only for the explicit owner fixture mode and can
    /// mint evidence, never a scheduled d74 record.
    pub fn install_a729_power_balance_hook() -> Result<InstalledHookAddress, String> {
        let _lifecycle = HOOK_LIFECYCLE_LOCK
            .lock()
            .map_err(|_| "A37 hook lifecycle mutex poisoned".to_string())?;
        if INSTALLED.load(Ordering::Acquire) {
            return Ok(InstalledHookAddress {
                post_refresh: MODULE_BASE.load(Ordering::Acquire) + POST_REFRESH_RVA as usize,
            });
        }
        let evidence = std::env::var_os("BATTLE_PLAN_A37_EVIDENCE").or_else(|| std::env::var_os("HOI4_A37_EVIDENCE")).as_deref()
            == Some(std::ffi::OsStr::new("1"));
        if !evidence
            && !(A729_CONTROLLER_OBSERVER_ACTIVATED
                && A729_PANEL_VISIBILITY_OBSERVER_ACTIVATED)
        {
            return Err("A37 hook install is disabled until the live fixture is accepted".into());
        }
        let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
        if base == 0 {
            return Err("A37 GetModuleHandleW(NULL) returned null".into());
        }
        let target = base + POST_REFRESH_RVA as usize;
        verify_sentinel(target)?;
        let original = unsafe {
            MinHook::create_hook(target as *mut c_void, hook_post_refresh as *mut c_void)
        }
        .map_err(|status| format!("A37 hook creation failed: {status}"))?;
        ORIGINAL_POST_REFRESH.store(original as usize, Ordering::Release);
        MODULE_BASE.store(base, Ordering::Release);
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            let _ = unsafe { MinHook::remove_hook(target as *mut c_void) };
            MODULE_BASE.store(0, Ordering::Release);
            ORIGINAL_POST_REFRESH.store(0, Ordering::Release);
            return Err(format!("A37 hook enable failed: {status}"));
        }
        INSTALLED.store(true, Ordering::Release);
        Ok(InstalledHookAddress { post_refresh: target })
    }

    /// Materialize and seal raw exact-epoch semantics against the latest hook
    /// callback and selected Present. Callers cannot supply an already-built
    /// PBP2 frame; every portable field is constructed by the receipt guard.
    /// This does not change either activation latch.
    pub fn seal_receipted_semantics(
        sample: &A729PostRefreshHookSample,
        semantic: A729SemanticBoundaryObservation,
        envelope: A729SemanticFrameEnvelope,
    ) -> Result<Vec<u8>, HookAdmissionError> {
        if !A729_CONTROLLER_OBSERVER_ACTIVATED
            || !A729_PANEL_VISIBILITY_OBSERVER_ACTIVATED
            || !crate::PRODUCTION_SCHEDULING_ENABLED
        {
            return Err(HookAdmissionError::Inactive);
        }
        let frame = materialize_receipted_semantics(sample, &semantic, &envelope)?;
        validate_frame(&frame).map_err(HookAdmissionError::Carrier)?;
        let mut state = state()
            .lock()
            .map_err(|_| HookAdmissionError::Materialization("A37 hook state mutex poisoned".into()))?;
        let Some(boundary) = state.latest_boundary.as_ref() else {
            state.rejected_frames = state.rejected_frames.saturating_add(1);
            return Err(HookAdmissionError::FrameJoinMismatch);
        };
        if boundary.sequence != semantic.callback_sequence
            || boundary.current_thread_id != sample.current_thread_id
            || boundary.view_ptr != sample.view_ptr
            || boundary.selected_record_ptr != sample.selected_record_ptr
            || boundary.selected_range_ptr != sample.selected_range_ptr
            || boundary.controller_open_raw != sample.controller_open_raw
            || boundary.controller_gui_root_ptr != sample.controller_gui_root_ptr
            || boundary.controller_gui_visibility_flags
                != sample.controller_gui_visibility_flags
            || boundary.full_refresh_raw != sample.full_refresh_raw
            || boundary.range_refresh_raw != sample.range_refresh_raw
            || boundary.materialization_qpc_ticks != sample.materialization_qpc_ticks
            || boundary.qpc_frequency_hz != sample.qpc_frequency_hz
            || boundary.current_value_raw != sample.controller_state.current_value_raw
            || boundary.change_raw != sample.controller_state.change_raw
        {
            state.rejected_frames = state.rejected_frames.saturating_add(1);
            return Err(HookAdmissionError::FrameJoinMismatch);
        }
        if state.sealed_frames.insert(frame.frame_hour, frame.clone()).is_some() {
            state.rejected_frames = state.rejected_frames.saturating_add(1);
            return Err(HookAdmissionError::FrameJoinMismatch);
        }
        encode_payload(&frame).map_err(HookAdmissionError::Carrier)
    }

    pub fn diagnostics() -> HookDiagnostics {
        let Ok(state) = state().lock() else {
            return HookDiagnostics {
                installed: INSTALLED.load(Ordering::Acquire),
                rejected_frames: 1,
                ..HookDiagnostics::default()
            };
        };
        HookDiagnostics {
            installed: INSTALLED.load(Ordering::Acquire),
            boundary_sequence: state.latest_boundary.as_ref().map_or(0, |row| row.sequence),
            sealed_frame_count: state.sealed_frames.len() as u64,
            rejected_boundaries: state.rejected_boundaries,
            rejected_frames: state.rejected_frames,
        }
    }

    pub fn uninstall_a729_power_balance_hook() -> Result<(), String> {
        let _lifecycle = HOOK_LIFECYCLE_LOCK
            .lock()
            .map_err(|_| "A37 hook lifecycle mutex poisoned".to_string())?;
        if !INSTALLED.load(Ordering::Acquire) {
            return Ok(());
        }
        let target = MODULE_BASE.load(Ordering::Acquire) + POST_REFRESH_RVA as usize;
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(())
            | Err(MH_STATUS::MH_ERROR_DISABLED)
            | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A37 hook disable failed: {status}")),
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while CALLBACKS_IN_FLIGHT.load(Ordering::Acquire) != 0 {
            if std::time::Instant::now() >= deadline {
                return Err("A37 callbacks did not quiesce".into());
            }
            std::thread::yield_now();
        }
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => return Err(format!("A37 hook removal failed: {status}")),
        }
        ORIGINAL_POST_REFRESH.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        INSTALLED.store(false, Ordering::Release);
        Ok(())
    }
}

#[cfg(windows)]
pub use installed_owner::{
    diagnostics, install_a729_power_balance_hook, latest_raw_controller_evidence,
    seal_receipted_semantics,
    uninstall_a729_power_balance_hook, A729PowerBalanceHookSource, HookDiagnostics,
    InstalledHookAddress, PostRefreshBoundaryReceipt, RawControllerEvidence,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> A729PostRefreshHookSample {
        A729PostRefreshHookSample {
            executable_sha256: BATTLE_PLAN_A729_EXECUTABLE_SHA256,
            producer_manifest_sha256: A729_CONTROLLER_EXACT20_MANIFEST_SHA256,
            panel_visibility_manifest_sha256: A729_PANEL_VISIBILITY_MANIFEST_SHA256,
            target_rva: POST_REFRESH_RVA,
            entry_sentinel: POST_REFRESH_ENTRY_SENTINEL,
            current_thread_id: 7,
            owner_thread_id: 7,
            view_ptr: 0x1000,
            view_vtable_rva: POWER_BALANCE_VIEW_VTABLE_RVA,
            selected_record_ptr: 0x2000,
            selected_range_ptr: 0x3000,
            controller_open_raw: 1,
            controller_gui_root_ptr: 0x4000,
            controller_gui_visibility_flags: GUI_CONTAINER_VISIBLE_MASK,
            full_refresh_raw: 1,
            range_refresh_raw: 0,
            materialization_qpc_ticks: 90,
            qpc_frequency_hz: 10_000_000,
            controller_state: A729ControllerStateObservation {
                current_value_raw: 35_000,
                change_raw: -1_000,
                left_side_id: "left".into(),
                right_side_id: "right".into(),
                trending_side_id: "right".into(),
                engine_range_indicator_frame: 2,
            },
        }
    }

    fn launcher_refresh_sample() -> A729LauncherVisibilityRefreshSample {
        A729LauncherVisibilityRefreshSample {
            executable_sha256: BATTLE_PLAN_A729_EXECUTABLE_SHA256,
            producer_manifest_sha256: A729_PANEL_VISIBILITY_MANIFEST_SHA256,
            target_rva: COUNTRY_POLITICS_REFRESH_RVA,
            entry_sentinel: COUNTRY_POLITICS_REFRESH_ENTRY_SENTINEL,
            current_thread_id: 7,
            owner_thread_id: 7,
            country_politics_view_ptr: 0x5000,
            country_politics_view_vtable_rva: COUNTRY_POLITICS_VIEW_VTABLE_RVA,
            country_politics_active_raw: 1,
            launcher_root_ptr: 0x6000,
            launcher_visibility_flags: GUI_CONTAINER_VISIBLE_MASK,
            selected_record_ptr: 0x2000,
            controller_ptr: 0x1000,
            controller_vtable_rva: POWER_BALANCE_VIEW_VTABLE_RVA,
            controller_open_raw: 1,
            controller_gui_root_ptr: 0x4000,
            controller_gui_visibility_flags: GUI_CONTAINER_VISIBLE_MASK,
            controller_selected_record_ptr: 0x2000,
        }
    }

    fn panel_toggle_sample() -> A729PanelToggleHookSample {
        A729PanelToggleHookSample {
            executable_sha256: BATTLE_PLAN_A729_EXECUTABLE_SHA256,
            producer_manifest_sha256: A729_PANEL_VISIBILITY_MANIFEST_SHA256,
            target_rva: COUNTRY_POLITICS_LAUNCHER_CLICK_RVA,
            entry_sentinel: COUNTRY_POLITICS_LAUNCHER_CLICK_ENTRY_SENTINEL,
            current_thread_id: 7,
            owner_thread_id: 7,
            country_politics_view_ptr: 0x5000,
            country_politics_view_vtable_rva: COUNTRY_POLITICS_VIEW_VTABLE_RVA,
            controller_ptr: 0x1000,
            controller_vtable_rva: POWER_BALANCE_VIEW_VTABLE_RVA,
            launcher_visible_before_raw: 1,
            panel_open_before_raw: 0,
            panel_open_after_raw: 1,
            controller_gui_root_ptr: 0x4000,
            controller_gui_visibility_flags_after: GUI_CONTAINER_VISIBLE_MASK,
            selected_record_ptr_after: 0x2000,
        }
    }

    #[test]
    fn exact_receipted_sample_normalizes_without_collapsing_three_leaves() {
        let admitted = validate_receipted_sample(&sample()).unwrap();
        assert!(admitted.panel_open);
        assert_eq!(admitted.controller_state.active_side_id, "right");
        assert_eq!(admitted.controller_state.trending_side_id, "right");
        assert_eq!(admitted.controller_state.bar_state, crate::BarState::MovingLeft);
        assert_eq!(admitted.controller_state.range_indicator_frame, 1);
    }

    #[test]
    fn live_admission_remains_inert() {
        assert_eq!(admit_live_sample(&sample()), Err(HookAdmissionError::Inactive));
        assert_eq!(
            after_live_admission::<()>(&sample(), || {
                panic!("inactive hook must not materialize")
            }),
            Err(HookAdmissionError::Inactive)
        );
    }

    #[test]
    fn sentinel_thread_owner_and_manifest_mismatches_fail_closed() {
        let mut bad = sample();
        bad.entry_sentinel[0] ^= 0xff;
        assert_eq!(
            validate_receipted_sample(&bad),
            Err(HookAdmissionError::SentinelMismatch)
        );

        let mut bad = sample();
        bad.owner_thread_id += 1;
        assert_eq!(
            validate_receipted_sample(&bad),
            Err(HookAdmissionError::ThreadMismatch)
        );

        let mut bad = sample();
        bad.view_vtable_rva += 8;
        assert_eq!(
            validate_receipted_sample(&bad),
            Err(HookAdmissionError::OwnerMismatch)
        );

        let mut bad = sample();
        bad.producer_manifest_sha256[0] ^= 0xff;
        assert_eq!(
            validate_receipted_sample(&bad),
            Err(HookAdmissionError::ManifestMismatch)
        );

        let mut bad = sample();
        bad.panel_visibility_manifest_sha256[0] ^= 0xff;
        assert_eq!(
            validate_receipted_sample(&bad),
            Err(HookAdmissionError::PanelVisibilityManifestMismatch)
        );

        let mut bad = sample();
        bad.controller_open_raw = 0;
        assert_eq!(
            validate_receipted_sample(&bad),
            Err(HookAdmissionError::PanelNotOpen)
        );
    }

    #[test]
    fn launcher_refresh_binds_selected_record_container_and_open_panel() {
        let admitted = validate_launcher_visibility_refresh(&launcher_refresh_sample()).unwrap();
        assert!(admitted.launcher_visible);
        assert!(admitted.panel_open);
        assert!(admitted.selected_balance_present);

        let mut hidden = launcher_refresh_sample();
        hidden.launcher_visibility_flags = 0;
        hidden.selected_record_ptr = 0;
        hidden.controller_open_raw = 0;
        hidden.controller_gui_visibility_flags = 0;
        hidden.controller_selected_record_ptr = 0x2000;
        let admitted = validate_launcher_visibility_refresh(&hidden).unwrap();
        assert_eq!(
            admitted,
            AuthenticatedA729PanelVisibilityObservation {
                launcher_visible: false,
                panel_open: false,
                selected_balance_present: false,
            }
        );
    }

    #[test]
    fn launcher_refresh_rejects_stale_or_unowned_visibility() {
        let mut bad = launcher_refresh_sample();
        bad.launcher_visibility_flags = 0;
        assert_eq!(
            validate_launcher_visibility_refresh(&bad),
            Err(HookAdmissionError::LifecycleMismatch)
        );

        let mut bad = launcher_refresh_sample();
        bad.country_politics_active_raw = 0;
        assert_eq!(
            validate_launcher_visibility_refresh(&bad),
            Err(HookAdmissionError::LifecycleMismatch)
        );

        let mut bad = launcher_refresh_sample();
        bad.controller_selected_record_ptr += 0x190;
        assert_eq!(
            validate_launcher_visibility_refresh(&bad),
            Err(HookAdmissionError::LifecycleMismatch)
        );
    }

    #[test]
    fn click_toggle_binds_controller_byte_to_gui_visibility() {
        let admitted = validate_panel_toggle(&panel_toggle_sample()).unwrap();
        assert!(admitted.panel_open);

        let mut close = panel_toggle_sample();
        close.panel_open_before_raw = 1;
        close.panel_open_after_raw = 0;
        close.controller_gui_visibility_flags_after = 0;
        let admitted = validate_panel_toggle(&close).unwrap();
        assert!(!admitted.panel_open);

        let mut stale = panel_toggle_sample();
        stale.controller_gui_visibility_flags_after = 0;
        assert_eq!(
            validate_panel_toggle(&stale),
            Err(HookAdmissionError::LifecycleMismatch)
        );
        assert_eq!(
            admit_live_panel_toggle(&panel_toggle_sample()),
            Err(HookAdmissionError::Inactive)
        );
        assert_eq!(
            admit_live_launcher_visibility_refresh(&launcher_refresh_sample()),
            Err(HookAdmissionError::Inactive)
        );
    }
}
