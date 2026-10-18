//! Canonical native read ranges shared by raw copies and dependency traces.

/// x86-64 canonical range check, identical to `engine_native::memory`'s gate.
///
/// Duplicated (not imported) so this crate needs no new public engine API;
/// the two implementations are proved equal by mirrored unit vectors in both
/// crates. Noncanonical sources fault with `ExceptionInformation[1] == -1`,
/// which `copy_fault` cannot attribute to the requested range. Gating here
/// records the same graceful fault the typed walk produces for an unreadable
/// source, before any guarded instruction runs. Empty spans succeed without
/// a check, matching `Memory`. The Rust gate makes the C `-1` path
/// unreachable through `take_inner`/`LiveMemory`; the C fix remains as
/// containment for direct FFI callers.
#[inline]
pub fn is_canonical_range(addr: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = addr.checked_add(len.wrapping_sub(1)) else {
        return false;
    };
    // Bit 47 is part of the sign extension, not an independent address bit.
    let high = addr >> 47;
    matches!(high, 0 | 0x1FFFF) && end >> 47 == high
}
