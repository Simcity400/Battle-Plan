//! CCapturedOperativeReference, walked by CIntelligenceAgency at Daily(16).

/// Captured-operative reference vtable.
pub const VTABLE_RVA: usize = 0x0296_b388;

/// Inline element stride.
pub const STRIDE: usize = 0x38;
/// Copied constructor argument word; its meaning is unknown.
pub const ARGUMENT_ID: usize = 0x08;
/// Full operative handle copied as two u32 words.
pub const OPERATIVE_HANDLE: usize = 0x0c;
/// Constructor copying the full operative handle.
pub const CONSTRUCTOR_RVA: usize = 0x01a2_15f0;
/// Handle-resolver candidate; its decode is unverified.
pub const HANDLE_RESOLVER_CANDIDATE_RVA: usize = 0x01a2_16c0;
