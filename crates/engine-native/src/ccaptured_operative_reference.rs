//! CCapturedOperativeReference, walked by CIntelligenceAgency at Daily(16).

/// Captured-operative reference vtable.
pub const VTABLE_RVA: usize = 0x0296_b388;

/// Inline element stride.
pub const STRIDE: usize = 0x38;
/// Full operative handle copied as two u32 words.
#[cfg(test)]
pub const OPERATIVE_HANDLE: usize = 0x0c;
/// Constructor copying the full operative handle.
pub const CONSTRUCTOR_RVA: usize = 0x01a2_15f0;
