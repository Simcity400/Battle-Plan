//! d13's change-signal gate: five entry detours on the runtime writers of
//! `[CTechnology]+0x170` (QN census `runs/qn_d13_census`, owner ruling
//! 2026-08-25) feeding a per-manager dirty table that the hourly technology
//! bracket reads passively. The constructor site is dominated by hooked
//! writers at all three of its call sites and installs nothing.
//!
//! Hook discipline follows the production registry contract exactly like
//! D95: MinHook REL5 primitives, install-time sentinel verification against
//! the pinned bytes, `StandardHookReceiptGroup::D13ChangeGate` custody, named
//! refusals, abort order, and byte-proving release receipts. The detour
//! bodies are a store into [`crate::gate`]'s dirty word - no locks, no
//! allocation, no logging - because this runs on the game's own threads.

pub mod gate;

#[cfg(windows)]
mod hooks_windows;

/// Cross-platform install predicate for callers outside the Windows-only
/// registry paths (the d13 walk and emitter run compiled on every target).
/// Off-Windows this is always false, which keeps the gate shut by the same
/// named path a refused install takes.
pub fn hooks_installed() -> bool {
    #[cfg(windows)]
    {
        hooks_windows::installed()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
pub use hooks_windows::{
    abort_partial_install_and_uninstall, install, installed, installed_hook_receipts,
    release_pending, take_released_hook_receipts, uninstall, HOOK_RECEIPT_COUNT,
    PRODUCTION_HOOK_SPECS,
};
