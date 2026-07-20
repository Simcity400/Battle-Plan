//! Inert exact-a729 post-refresh observer scaffold for H-A37.
//!
//! The exact20 receipt resolves `CPowerBalanceView`'s terminal updater as
//! `void(this, u8 full_refresh, u8 range_refresh)` at RVA `0x01C4BAA0` and
//! supplies its entry bytes. This module authenticates a raw callback sample;
//! it does not install a detour while the separate live activation gate is
//! false. Pointer-bearing owner fields are consumed only by the guard and are
//! never returned or serialized.

use crate::{
    encode_payload, normalize_a729_controller_state, A729ControllerStateObservation, CarrierError,
    NormalizedControllerState, PowerBalancePresentationFrame,
    A729_CONTROLLER_EXACT20_AUTHORITY_BOUND,
    A729_CONTROLLER_EXACT20_MANIFEST_SHA256, A729_CONTROLLER_OBSERVER_ACTIVATED,
};

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
    pub controller_state: A729ControllerStateObservation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedA729PostRefreshObservation {
    pub panel_open: bool,
    pub full_refresh: bool,
    pub range_refresh: bool,
    pub controller_state: NormalizedControllerState,
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

/// Inert source scaffold for the original-call-then-observe detour wrapper.
/// The materializer is never invoked while activation is false. Once enabled,
/// its complete pointer-free frame must agree mechanically with the normalized
/// controller leaves before the sole PBP2 encoder is reached.
pub fn encode_live_post_refresh<F>(
    sample: &A729PostRefreshHookSample,
    materialize: F,
) -> Result<Vec<u8>, HookAdmissionError>
where
    F: FnOnce(
        &AuthenticatedA729PostRefreshObservation,
    ) -> Result<PowerBalancePresentationFrame, String>,
{
    let authenticated = admit_live_sample(sample)?;
    let frame = materialize(&authenticated).map_err(HookAdmissionError::Materialization)?;
    if frame.producer_manifest_sha256 != A729_CONTROLLER_EXACT20_MANIFEST_SHA256
        || frame.balances.len() != 1
    {
        return Err(HookAdmissionError::FrameJoinMismatch);
    }
    let balance = &frame.balances[0];
    if balance.active_side_id != authenticated.controller_state.active_side_id
        || balance.trending_side_id != authenticated.controller_state.trending_side_id
        || balance.bar_state != authenticated.controller_state.bar_state
        || balance.range_indicator_frame
            != authenticated.controller_state.range_indicator_frame
    {
        return Err(HookAdmissionError::FrameJoinMismatch);
    }
    encode_payload(&frame).map_err(HookAdmissionError::Carrier)
}

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
            encode_live_post_refresh(&sample(), |_| panic!("inactive hook must not materialize")),
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
