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

pub const BATTLE_PLAN_A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
pub const POST_REFRESH_RVA: u32 = 0x01c4_baa0;
pub const POWER_BALANCE_VIEW_VTABLE_RVA: u32 = 0x02a4_21c0;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A729PostRefreshHookSample {
    pub executable_sha256: [u8; 32],
    pub producer_manifest_sha256: [u8; 32],
    pub target_rva: u32,
    pub entry_sentinel: [u8; 17],
    pub current_thread_id: u32,
    pub owner_thread_id: u32,
    pub view_ptr: usize,
    pub view_vtable_rva: u32,
    pub selected_record_ptr: usize,
    pub selected_range_ptr: usize,
    pub full_refresh_raw: u8,
    pub range_refresh_raw: u8,
    pub controller_state: A729ControllerStateObservation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedA729PostRefreshObservation {
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
    TargetMismatch,
    SentinelMismatch,
    ThreadMismatch,
    OwnerMismatch,
    InvalidFlags,
    InvalidControllerState(CarrierError),
    Materialization(String),
    FrameJoinMismatch,
    Carrier(CarrierError),
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
    {
        return Err(HookAdmissionError::OwnerMismatch);
    }
    if sample.full_refresh_raw > 1 || sample.range_refresh_raw > 1 {
        return Err(HookAdmissionError::InvalidFlags);
    }
    let controller_state = normalize_a729_controller_state(&sample.controller_state)
        .map_err(HookAdmissionError::InvalidControllerState)?;
    Ok(AuthenticatedA729PostRefreshObservation {
        full_refresh: sample.full_refresh_raw != 0,
        range_refresh: sample.range_refresh_raw != 0,
        controller_state,
    })
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
            target_rva: POST_REFRESH_RVA,
            entry_sentinel: POST_REFRESH_ENTRY_SENTINEL,
            current_thread_id: 7,
            owner_thread_id: 7,
            view_ptr: 0x1000,
            view_vtable_rva: POWER_BALANCE_VIEW_VTABLE_RVA,
            selected_record_ptr: 0x2000,
            selected_range_ptr: 0x3000,
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

    #[test]
    fn exact_receipted_sample_normalizes_without_collapsing_three_leaves() {
        let admitted = validate_receipted_sample(&sample()).unwrap();
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
    }
}
