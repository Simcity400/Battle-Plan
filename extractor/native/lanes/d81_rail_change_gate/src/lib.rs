//! d81's RAIL WRITE-TAP: three entry detours on the identified writers of the
//! `CProvinceRailwayInfo` pair vector, feeding one process-wide seqlock witness
//! that d81's hourly copy bracket reads passively.
//!
//! WHY A HOOK IS LAWFUL HERE. Lane TD's dossier
//! (`runs/td_rail_change_signal/DOSSIER.md`) is the whole-image search the
//! owner's minimise-hooks ruling (2026-08-24) requires, and it found no passive
//! change signal: the rail level is written IN PLACE at a stable address inside
//! a stable allocation reached through a table whose base never moves (§4 M2a),
//! the engine's own graphics consumer re-compares the pair vector element by
//! element rather than reading a version word (§6.1), and the one rail-specific
//! notification the engine keeps is cleared by its own producer inside a single
//! update, so an hourly out-of-band reader cannot use it (§6.2). Its §6.6
//! states what that search could NOT establish -- it is not a proof that no
//! signal exists anywhere, and its writer census is not closed -- and this
//! crate carries the residual at that strength rather than rounding it up. The
//! structural-perf ruling of 2026-08-27 approves write-tap hooks on that
//! footing.
//!
//! WHAT THE CONSUMER DOES WITH IT. `attach_log::d81_vector_store` copies nine
//! scattered families per frame; three of its eleven stores are the rails
//! (`RailHeader`, its closing image, and `PairBlob`). It takes a
//! [`gate::RailWitness`] on either side of that copy and remembers the result
//! only when the pair proves no hooked writer overlapped it; a later frame
//! reuses those three stores verbatim only while the witness is still current
//! AND the frame's own re-derived `(province_id, info_pointer)` list is
//! identical to the remembered one. Everything else about the frame is unchanged:
//! the bulk copy still runs, the other families are still copied, the
//! unchanged-geometry gate still compares every byte, and the hour d81
//! publishes is still the hour it observed.
//!
//! Hook discipline follows the production registry contract exactly like D13's
//! change gate (the WITNESS MODEL does not: see `gate`'s module comment for why
//! d13's two-window dirty pair is unsound for a writer that spans two
//! brackets). MinHook REL5 primitives, install-time sentinel verification
//! against the pinned bytes of the hash-verified a729 instruction artifacts,
//! `StandardHookReceiptGroup::D81RailChangeGate` custody, named refusals,
//! reverse abort order, and byte-proving release receipts. A refused install
//! degrades to the full copy on every frame — the pre-tap program — and never
//! to missing data.

pub mod gate;

#[cfg(windows)]
mod hooks_windows;

/// Cross-platform install predicate for callers outside the Windows-only
/// registry paths (d81's copy bracket compiles on every target). Off-Windows
/// this is always false, which keeps the gate shut by the same named path a
/// refused install takes.
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

/// `[marks_total, windows_opened]`, or zeros where the detours cannot exist.
/// `marks_total` counts MARK OPERATIONS -- two per completed hooked call --
/// not calls; see [`gate::mark`].
pub fn gate_census() -> [u64; 2] {
    #[cfg(windows)]
    {
        hooks_windows::gate_census()
    }
    #[cfg(not(windows))]
    {
        [gate::marks_total(), gate::windows_opened()]
    }
}

#[cfg(windows)]
pub use hooks_windows::{
    abort_partial_install_and_uninstall, install, installed, installed_hook_receipts,
    release_pending, take_released_hook_receipts, uninstall, HOOK_RECEIPT_COUNT,
    PRODUCTION_HOOK_SPECS,
};
