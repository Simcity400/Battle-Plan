//! CDecisionStatus state containers retained from the old decision walker.
//! Array headers use CPdxArray's pointer/u32 capacity/u32 count layout.

/// CDecisionStatus primary vtable; offsets.rs:287, confirmed by pinned-exe RTTI and validated_decision_status_ptr.
pub const VTABLE_RVA: usize = 0x027d_50f0;
/// CDecisionStatus owner CCountry pointer; offsets.rs:279, validated_decision_status_ptr compares it with the walked country.
pub const COUNTRY: usize = 0x08;
/// CDecisionStatus decisions-taken array; build_profile.rs:807, decision_status_present at attach_log.rs:171183 retained only its count.
pub const DECISIONS_TAKEN: usize = 0x40;
/// CDecisionStatus re-enable CDecisionCooldown pointer array; build_profile.rs:808, read_decision_cooldowns at attach_log.rs:171241.
pub const RE_ENABLE: usize = 0x58;
/// CDecisionStatus removal CDecisionCooldown pointer array; build_profile.rs:809, read_decision_cooldowns at attach_log.rs:171241.
pub const REMOVE: usize = 0x70;
/// CDecisionStatus active CTimedDecision pointer array; offsets.rs:284, read_active_timed_decisions at attach_log.rs:171215.
pub const ACTIVE_TIMED: usize = 0xa0;
/// CDecisionStatus timed CTargetedDecision pointer array; build_profile.rs:795, read_country_decisions at attach_log.rs:171139.
pub const TARGETED_TIMED: usize = 0xe8;
/// CDecisionStatus CTargetedDecision pointer array; build_profile.rs:796, read_country_decisions at attach_log.rs:171130.
pub const TARGETED: usize = 0x100;
/// CDecisionStatus timed/targeted/cooldown array element stride; attach_log.rs:171263 and 171340 dereference eight-byte pointers.
pub const RECORD_POINTER_STRIDE: usize = 8;

/// CDecisionStatus ordinary-taken entry FUN140728bc0; direct_frd_transitions_live/lib.rs:57 and hook_ordinary take status and definition pointers.
pub const ORDINARY_TAKEN_RVA: usize = 0x0072_8bc0;
/// FUN140728bc0 entry prefix; direct_frd_transitions_live/a729_hook.rs:60-62 bound and checked these bytes before its excluded hook.
pub const ORDINARY_TAKEN_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x56, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf2, 0x48,
];
/// CDecisionStatus targeted-taken entry FUN140728ce0; direct_frd_transitions_live/lib.rs:58 and hook_targeted retain an opaque target argument.
pub const TARGETED_TAKEN_RVA: usize = 0x0072_8ce0;
/// FUN140728ce0 entry prefix; direct_frd_transitions_live/a729_hook.rs:63-65 bound and checked these bytes before its excluded hook.
pub const TARGETED_TAKEN_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// CDecisionStatus timed-activation entry FUN140728fb0; direct_frd_transitions_live/lib.rs:59 and hook_timed take status and definition pointers.
pub const TIMED_ACTIVATED_RVA: usize = 0x0072_8fb0;
/// FUN140728fb0 entry prefix; direct_frd_transitions_live/a729_hook.rs:66-68 bound and checked these bytes before its excluded hook.
pub const TIMED_ACTIVATED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];
/// Decision-lifecycle correlation candidate FUN140723060; direct_frd_transitions_live/lib.rs:60 declares it only, with no ABI or sentinel proof.
pub const LIFECYCLE_CORRELATION_CANDIDATE_RVA: usize = 0x0072_3060;
