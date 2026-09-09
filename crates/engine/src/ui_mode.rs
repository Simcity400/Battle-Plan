//! Current UI mode scalar; the old reader does not establish an enclosing C++ class.

/// Pointer to the current UI-mode i32; diplomacy_intel_operatives_live/src/a30_visibility.rs:532-541 dereferences image+0x3324028 then reads i32.
pub const POINTER_RVA: usize = 0x0332_4028;
/// Agency UI mode value; a30_visibility.rs:12,426-467 uses engine mode 3 to select agency visibility.
pub const AGENCY: i32 = 3;
/// Intelligence ledger UI mode value; a30_visibility.rs:13,426-456 uses engine mode 0x1E to select the ledger.
pub const INTEL_LEDGER: i32 = 0x1e;
