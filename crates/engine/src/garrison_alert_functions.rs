//! Garrison-alert predicate metadata; no detour is installed here.
//! Selected-human scope and rounded whole-equipment shortages are presentation
//! semantics, separate from exact hourly country state.

/// Garrison alert predicate.
pub const PRODUCER_RVA: usize = 0x00af_e8a0;
/// Garrison-alert entry bytes.
pub const PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
