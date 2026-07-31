//! The one way an inline detour is enabled in this process.
//!
//! # The invariant
//!
//! **No inline detour is ever enabled in a module that could still be
//! unloaded.**
//!
//! A detour body lives in this DLL. Once a target is patched, a game thread can
//! branch into that body and be preempted before it executes a single
//! instruction, and nothing observable distinguishes that thread from no
//! thread -- no counter reading, no flag inside the body, because the thread is
//! already committed. If the module is then unloaded, that thread resumes on
//! unmapped memory and the game faults. Worse, if a later injection reuses the
//! image base first, the thread runs the NEW mapping's detour against its
//! freshly zeroed state and returns without calling the engine's own function,
//! which changes what the game computes.
//!
//! Retaining a MinHook trampoline does not help with any of that: the
//! trampoline is a separate allocation, and the code the poised thread is
//! actually inside is ours.
//!
//! So [`enable`] pins the containing module BEFORE it patches anything, and
//! refuses to patch at all if the pin fails. `GET_MODULE_HANDLE_EX_FLAG_PIN`
//! makes the loader reference permanent: `FreeLibrary` on any other reference
//! cannot bring the count to zero, so the mapping survives for the life of the
//! process whatever the collector or the host subsequently do.
//!
//! # Why this is a crate and not a helper
//!
//! Twenty-two crates own inline detours here, and only ten of them route
//! through `production_hook_receipt`. An invariant enforced in the receipt path
//! would cover under a third of the exposure while reading as though it covered
//! all of it -- which is worse than not having it, because the gap is invisible.
//! This crate is the single choke point, it is cheap enough for every lane to
//! depend on, and `no_direct_minhook_enable_outside_this_crate` fails the suite
//! if any crate reaches past it.
//!
//! # What this does NOT establish
//!
//! It does not prove no thread is resident in a detour. Nothing available here
//! can; that is the premise the whole design starts from. What it does is make
//! residency SURVIVABLE instead of fatal -- the code stays mapped, so a thread
//! that resumes late runs the real body against real state.
//!
//! It says nothing about the target's bytes. A pinned module with a detour
//! still patched in is not a released hook; restoring the target is a separate
//! obligation each lane keeps, and the host's teardown proof checks it
//! independently.
//!
//! That the pinned mapping actually survives a `FreeLibrary` is a property of
//! the Windows loader, asserted here only as "the PIN flag was accepted". No
//! offline test in this crate can observe a real unload attempt; that needs a
//! live patched `hoi4.exe`.
//!
//! # Cost
//!
//! One `GetModuleHandleExW` on the first enable of the process, and one relaxed
//! atomic load per enable after that. `enable` is an install-time call; no
//! producer path reaches it. The permanent cost is the mapping itself, which is
//! the deliberate trade this module exists to make.

#[cfg(windows)]
mod windows_impl {
    use core::ffi::c_void;
    use minhook::{MinHook, MH_STATUS};
    use std::fmt;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    };

    /// Why an inline detour was not enabled.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum InlineDetourEnableError {
        /// The containing module could not be pinned, so nothing was patched.
        /// Carries `GetLastError` from the refused `GetModuleHandleExW`.
        ModulePinRefused(u32),
        /// The pin succeeded and MinHook refused the patch.
        MinHook(MH_STATUS),
    }

    impl fmt::Display for InlineDetourEnableError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::ModulePinRefused(code) => write!(
                    f,
                    "inline detour refused: containing module could not be pinned \
                     against unload (GetLastError {code})"
                ),
                Self::MinHook(status) => write!(f, "{status}"),
            }
        }
    }

    impl std::error::Error for InlineDetourEnableError {}

    impl InlineDetourEnableError {
        /// The underlying MinHook status, when the refusal came from MinHook
        /// rather than from the pin. Call sites that inspect specific statuses
        /// use this; call sites that only render the failure use `Display`.
        pub fn minhook_status(self) -> Option<MH_STATUS> {
            match self {
                Self::MinHook(status) => Some(status),
                Self::ModulePinRefused(_) => None,
            }
        }
    }

    static MODULE_PINNED: AtomicBool = AtomicBool::new(false);
    static ENABLED_DETOURS: AtomicU64 = AtomicU64::new(0);

    /// A function whose address is inside this module, used only to name the
    /// module to pin. `#[inline(never)]` so it cannot be folded away into a
    /// caller in some other module.
    #[inline(never)]
    extern "system" fn residency_anchor() {}

    /// Take a permanent loader reference on the module containing this code.
    ///
    /// Idempotent and racy-safe: two threads pinning concurrently both succeed,
    /// and the second `GET_MODULE_HANDLE_EX_FLAG_PIN` is a no-op the loader
    /// already satisfied. The flag is what matters, not the returned handle,
    /// which is deliberately discarded.
    fn pin_containing_module() -> Result<(), u32> {
        if MODULE_PINNED.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut module = ptr::null_mut();
        let pinned = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                residency_anchor as *const () as *const u16,
                &mut module,
            )
        };
        if pinned == 0 || module.is_null() {
            return Err(unsafe { GetLastError() });
        }
        MODULE_PINNED.store(true, Ordering::Release);
        Ok(())
    }

    /// Enable an inline detour at `target`, pinning this module first.
    ///
    /// FAIL-CLOSED and the order is the whole point: if the module cannot be
    /// pinned, the target is not patched, so the state where a detour is live
    /// in an unloadable module never exists. Patching first and pinning second
    /// would leave exactly that window open.
    ///
    /// # Safety
    ///
    /// `target` must name a hook already created with MinHook, and the caller
    /// keeps every obligation `MH_EnableHook` has: the detour must remain a
    /// valid callable for as long as the patch is live.
    pub unsafe fn enable(target: *mut c_void) -> Result<(), InlineDetourEnableError> {
        pin_containing_module().map_err(InlineDetourEnableError::ModulePinRefused)?;
        match unsafe { MinHook::enable_hook(target) } {
            Ok(()) => {
                ENABLED_DETOURS.fetch_add(1, Ordering::AcqRel);
                Ok(())
            }
            Err(status) => Err(InlineDetourEnableError::MinHook(status)),
        }
    }

    /// True once this module holds a permanent loader reference because an
    /// inline detour was enabled through [`enable`].
    ///
    /// The collector's unload path consults this. It is deliberately one-way:
    /// there is no clearing it, because a cleared flag would describe a module
    /// that is safe to unload while a thread may still be inside a detour body.
    pub fn module_pinned_against_unload() -> bool {
        MODULE_PINNED.load(Ordering::Acquire)
    }

    /// How many inline detours this module has enabled. Diagnostic; the census
    /// the teardown proof reports alongside the pin.
    pub fn enabled_detour_count() -> u64 {
        ENABLED_DETOURS.load(Ordering::Acquire)
    }
}

#[cfg(windows)]
pub use windows_impl::{
    enable, enabled_detour_count, module_pinned_against_unload, InlineDetourEnableError,
};

/// The banned MinHook entry points, spelled as they appear in a call.
///
/// Kept as separate fragments and joined at runtime so that this list does not
/// itself match the scan, which would make the guard trivially self-satisfying.
#[cfg(test)]
const BANNED_CALL_FRAGMENTS: &[(&str, &str)] = &[
    ("MinHook::", "enable_hook("),
    ("MinHook::", "enable_all_hooks("),
    ("MinHook::", "queue_enable_hook("),
];

/// Collapse whitespace around `::` so `MinHook :: enable_hook` cannot slip past
/// a substring match, and normalise any local alias of `MinHook` back to the
/// real name so `use minhook::MinHook as MH; MH::enable_hook(..)` is caught too.
///
/// Deliberately crude rather than a real parser: a parser would be a second
/// thing to get wrong, and every bypass this normalisation does not catch --
/// a macro that assembles the path, the raw `minhook::ffi` symbol, a different
/// hooking library -- requires an edit that is visible in review. What is
/// closed here is the reachable-today path of writing the call.
#[cfg(test)]
fn normalize_for_scan(text: &str) -> String {
    let mut out = text.to_string();
    while out.contains(" ::") || out.contains(":: ") {
        out = out.replace(" ::", "::").replace(":: ", "::");
    }
    let mut aliases = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("use minhook::MinHook as ") {
            if let Some(alias) = rest.split(|c: char| c == ';' || c.is_whitespace()).next() {
                if !alias.is_empty() {
                    aliases.push(alias.to_string());
                }
            }
        }
    }
    for alias in aliases {
        out = out.replace(&format!("{alias}::"), "MinHook::");
    }
    out
}

/// Every line of `text` that enables an inline detour without the wrapper.
#[cfg(test)]
fn scan_source_for_direct_enable(text: &str) -> Vec<usize> {
    let normalized = normalize_for_scan(text);
    let banned: Vec<String> = BANNED_CALL_FRAGMENTS
        .iter()
        .map(|(prefix, call)| format!("{prefix}{call}"))
        .collect();
    normalized
        .lines()
        .enumerate()
        .filter(|(_, line)| banned.iter().any(|b| line.contains(b.as_str())))
        .map(|(index, _)| index + 1)
        .collect()
}

#[cfg(test)]
mod source_ban_tests {
    use super::{scan_source_for_direct_enable, BANNED_CALL_FRAGMENTS};
    use std::path::{Path, PathBuf};

    /// Every `.rs` file in the extractor workspace except this crate's own.
    fn workspace_sources() -> Vec<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("inline_detour has a parent directory")
            .to_path_buf();
        assert!(
            root.join("Cargo.toml").is_file(),
            "expected the extractor workspace root at {}",
            root.display()
        );
        let mut sources = Vec::new();
        let mut stack = vec![root.clone()];
        let skip = ["target", ".cargo-target", ".git", "runs", "fixtures"];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !skip.contains(&name) {
                        stack.push(path);
                    }
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    sources.push(path);
                }
            }
        }
        assert!(
            sources.len() > 100,
            "the scan found only {} sources; it is not reaching the workspace",
            sources.len()
        );
        sources
    }

    fn is_this_crate(path: &Path) -> bool {
        path.starts_with(env!("CARGO_MANIFEST_DIR"))
    }

    /// THE ENFORCEMENT. Fails if any crate patches a target without going
    /// through [`super::enable`].
    ///
    /// A detour enabled outside this crate is a detour enabled in a module that
    /// was never pinned, so the collector can later unload itself out from under
    /// a thread poised in that detour's body. Twelve of the twenty-two crates
    /// that own detours have no other reason to depend on the receipt crate, so
    /// there is no smaller choke point that covers them.
    ///
    /// Deliberately a source scan: MinHook is a third-party crate any workspace
    /// member can name directly, so nothing in Rust's visibility rules can stop
    /// a new lane calling it. The tree already uses source scanning for ordering
    /// laws it cannot otherwise state (`division_combat_stats_live`).
    ///
    /// SCOPE, stated because it is narrower than the invariant: this walks
    /// `extractor/native` only. `replay_interface/native/terrain_capture` is a
    /// separately injected DLL with its own MinHook detours and its own pin,
    /// and it is NOT covered here or by `inline_detour::enable`.
    #[test]
    fn no_direct_minhook_enable_outside_this_crate() {
        let mut violations = Vec::new();
        for path in workspace_sources() {
            if is_this_crate(&path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in scan_source_for_direct_enable(&text) {
                violations.push(format!("{}:{}", path.display(), line));
            }
        }
        assert!(
            violations.is_empty(),
            "inline detours must be enabled through `inline_detour::enable`, which pins this module against unload first. Enabling directly leaves a detour live in a module the collector can still unload, and a thread poised in that detour body then resumes on unmapped memory.\n{}",
            violations.join("\n")
        );
    }

    /// THE GUARD ON THE GUARD, and it now runs the real scanner.
    ///
    /// The previous version asserted that a planted string contained the banned
    /// substring, which is a property of the string and not of the scanner:
    /// deleting the violation-collection branch left it green. This feeds
    /// planted sources through `scan_source_for_direct_enable` itself.
    ///
    /// The alias and whitespace cases are here because they were real bypasses:
    /// `use minhook::MinHook as MH; MH::enable_hook(t)` and
    /// `MinHook :: enable_hook(t)` both defeated the plain substring match the
    /// first version used, and every lane already depends on MinHook so neither
    /// needs a new dependency to reach.
    #[test]
    fn the_scanner_catches_direct_aliased_and_whitespace_split_calls() {
        let (prefix, call) = BANNED_CALL_FRAGMENTS[0];
        let direct = format!("fn f() {{ unsafe {{ {prefix}{call}t) }}; }}");
        assert_eq!(
            scan_source_for_direct_enable(&direct),
            vec![1],
            "the plain call must be caught"
        );

        let spaced = format!("fn f() {{ unsafe {{ MinHook :: {call}t) }}; }}");
        assert_eq!(
            scan_source_for_direct_enable(&spaced),
            vec![1],
            "whitespace around :: must not defeat the scan"
        );

        let aliased = format!(
            "use minhook::MinHook as MH;\nfn f() {{ unsafe {{ MH::{call}t) }}; }}"
        );
        assert_eq!(
            scan_source_for_direct_enable(&aliased),
            vec![2],
            "an aliased import must not defeat the scan"
        );

        let clean = "fn f() { unsafe { inline_detour::enable(t) }; }";
        assert!(
            scan_source_for_direct_enable(clean).is_empty(),
            "the wrapper call is not a violation"
        );
    }

    /// The walk must actually reach the crates that own detours; if it ever
    /// stopped, every future violation would pass unnoticed.
    #[test]
    fn the_scan_reaches_the_crates_that_own_detours() {
        let sources = workspace_sources();
        for owner in [
            "fow_visibility_live",
            "land_combat_detail_events_live",
            "dynamic_collection_trace_live",
            "collector",
        ] {
            assert!(
                sources.iter().any(|p| p.to_string_lossy().contains(owner)),
                "the scan must reach {owner}, which owns inline detours"
            );
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Falsifiable: fails if the pin stops happening, or stops being recorded.
    ///
    /// The pin is what makes every other retention claim in this workspace
    /// unconditional rather than "while the DLL happens to still be mapped", so
    /// a build that dropped it would silently reopen the use-after-free the
    /// whole design is arranged around.
    ///
    /// What this CANNOT show, in those words: that a pinned module actually
    /// survives a `FreeLibrary`. That is the Windows loader's behaviour and
    /// observing it needs a live patched `hoi4.exe` with a real unload attempt.
    /// What is asserted here is that the PIN flag was accepted for this exact
    /// module before any detour was enabled.
    #[test]
    fn enabling_a_detour_pins_the_containing_module_first() {
        // No detour exists in a test process, so `enable` will be refused by
        // MinHook -- AFTER the pin, which is the ordering under test.
        let refusal = unsafe { enable(0x1000 as *mut core::ffi::c_void) }
            .expect_err("no hook was ever created at this address");
        assert!(
            refusal.minhook_status().is_some(),
            "the refusal must come from MinHook, not from the pin: {refusal}"
        );
        assert!(
            module_pinned_against_unload(),
            "the module must be pinned before any patch is attempted"
        );
    }

    /// Falsifiable: fails if a MinHook refusal is ever counted as an enabled
    /// detour. The count is what the teardown proof reports as the reason the
    /// module is retained, so inflating it would claim residency that never
    /// existed.
    #[test]
    fn a_refused_patch_is_not_counted_as_an_enabled_detour() {
        let before = enabled_detour_count();
        let _ = unsafe { enable(0x2000 as *mut core::ffi::c_void) };
        assert_eq!(
            enabled_detour_count(),
            before,
            "a detour MinHook refused to patch was never enabled"
        );
    }

    /// Falsifiable: fails if the pin-refusal and MinHook-refusal arms ever
    /// render identically. They are different facts with different operator
    /// responses -- one means the loader refused us, the other means the target
    /// was not hookable -- and a teardown reading one as the other would look
    /// in the wrong place.
    #[test]
    fn the_two_refusals_are_distinguishable() {
        let pin = InlineDetourEnableError::ModulePinRefused(5);
        assert!(pin.minhook_status().is_none());
        assert!(pin.to_string().contains("could not be pinned"));
        assert!(pin.to_string().contains('5'));
        let hook = InlineDetourEnableError::MinHook(minhook::MH_STATUS::MH_ERROR_NOT_CREATED);
        assert!(hook.minhook_status().is_some());
        assert!(!hook.to_string().contains("could not be pinned"));
        assert_ne!(pin.to_string(), hook.to_string());
    }
}
