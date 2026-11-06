//! CCapturedOperativeReference, walked by CIntelligenceAgency at Daily(16).

/// Captured-operative reference vtable.
pub const VTABLE_RVA: usize = 0x029f_e4a0;

/// Inline element stride.
pub const STRIDE: usize = 0x38;
/// Full operative handle copied as two u32 words.
#[cfg(test)]
pub const OPERATIVE_HANDLE: usize = 0x0c;
/// Constructor copying the full operative handle.
pub const CONSTRUCTOR_RVA: usize = 0x01ac_7bb0;
