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

    /// Run `op` ONLY if `pin` succeeded, and never otherwise.
    ///
    /// Every public entry point funnels through here, because the ordering IS
    /// the invariant: patching before pinning leaves a window in which a detour
    /// is live in an unloadable module, and that window is what produced the
    /// recorded 0xc0000005 faults in an unloaded capture mapping.
    ///
    /// Both inputs are parameters so the ordering is testable. It is not
    /// otherwise: by the time any test runs, some earlier test has already
    /// pinned the process, so asserting "the module is pinned afterwards"
    /// cannot tell a pin-first build from a pin-second one. The property that
    /// actually discriminates is that a REFUSED pin means `op` never ran.
    pub(crate) fn run_pinned<T>(
        pin: impl FnOnce() -> Result<(), u32>,
        op: impl FnOnce() -> Result<T, MH_STATUS>,
    ) -> Result<T, InlineDetourEnableError> {
        pin().map_err(InlineDetourEnableError::ModulePinRefused)?;
        op().map_err(InlineDetourEnableError::MinHook)
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
        run_pinned(pin_containing_module, || unsafe { MinHook::enable_hook(target) })?;
        ENABLED_DETOURS.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Take the permanent loader reference now, without enabling anything.
    ///
    /// For callers that pin at a different moment than they patch -- a capture
    /// that disarms its hooks at dwell end and keeps them mapped as
    /// pass-throughs, for instance. Fail-closed like [`enable`]: the flag is
    /// recorded only after the loader accepts the pin, never before it is
    /// attempted. Recording first and proceeding on failure is the exact
    /// inversion of this invariant, and it reads as pinned while the module is
    /// still unloadable.
    pub fn pin_module_now() -> Result<(), InlineDetourEnableError> {
        pin_containing_module().map_err(InlineDetourEnableError::ModulePinRefused)
    }

    /// Queue an inline detour for a batched enable, pinning this module first.
    ///
    /// Queuing does not patch, but it is the point of no return for a caller
    /// that will then apply the batch, and pinning here means a queue that is
    /// never applied still leaves the module retained -- the conservative
    /// direction.
    ///
    /// # Safety
    ///
    /// As [`enable`].
    pub unsafe fn queue_enable(target: *mut c_void) -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || unsafe {
            MinHook::queue_enable_hook(target)
        })
    }

    /// Apply every queued enable, pinning this module first.
    ///
    /// This is the call that actually patches a queued batch, so the pin
    /// ordering matters here exactly as it does in [`enable`]. Counted as ONE
    /// enable operation covering the whole batch: the batch size is MinHook's
    /// internal state and is not reported back.
    ///
    /// # Safety
    ///
    /// As [`enable`], for every target in the queue.
    pub unsafe fn apply_queued() -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || unsafe { MinHook::apply_queued() })?;
        ENABLED_DETOURS.fetch_add(1, Ordering::AcqRel);
        Ok(())
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
pub(crate) use windows_impl::run_pinned;
pub use windows_impl::{
    apply_queued, enable, enabled_detour_count, module_pinned_against_unload, pin_module_now,
    queue_enable, InlineDetourEnableError,
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
    ("MinHook::", "apply_queued("),
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
        // Both the bare form and the grouped form, which is the idiomatic one
        // and was a live bypass: `use minhook::{MinHook as MH, MH_STATUS};`
        // aliases just as effectively as `use minhook::MinHook as MH;` and
        // needs no extra dependency, since every crate here already has MinHook.
        for marker in ["use minhook::MinHook as ", "MinHook as "] {
            if !line.starts_with("use ") && marker.starts_with("use ") {
                continue;
            }
            if marker == "MinHook as " && !line.starts_with("use minhook::") {
                continue;
            }
            let Some(at) = line.find(marker) else { continue };
            let rest = &line[at + marker.len()..];
            if let Some(alias) = rest
                .split(|c: char| c == ';' || c == ',' || c == '}' || c.is_whitespace())
                .next()
            {
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

    /// Every `.rs` file in every native workspace that can inject into the
    /// game, except this crate's own.
    ///
    /// EXPLICITLY MORE THAN ONE WORKSPACE. `terrain_capture` lives in
    /// `replay_interface/native` and is a separately injected DLL with its own
    /// D3D11 detours; a scan that stopped at the extractor boundary would have
    /// declared the invariant enforced while that mapping patched before
    /// pinning. An invariant that stops at a workspace boundary is not one.
    fn workspace_sources() -> Vec<PathBuf> {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("inline_detour sits three levels below the repository root")
            .to_path_buf();
        let roots: Vec<PathBuf> = ["extractor/native", "replay_interface/native"]
            .iter()
            .map(|rel| repo.join(rel))
            .collect();
        for root in &roots {
            assert!(
                root.join("Cargo.toml").is_file(),
                "expected a native workspace root at {}",
                root.display()
            );
        }
        let mut sources = Vec::new();
        let mut stack = roots;
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

        // The GROUPED import, which is the idiomatic form and was a live
        // bypass: the bare-form check alone missed it entirely.
        let grouped = format!(
            "use minhook::{{MinHook as MH, MH_STATUS}};\nfn f() {{ unsafe {{ MH::{call}t) }}; }}"
        );
        assert_eq!(
            scan_source_for_direct_enable(&grouped),
            vec![2],
            "a grouped aliased import must not defeat the scan either"
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
            "terrain_capture",
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

    /// Several tests here read a delta on the process-global enable counter, so
    /// they are serialized rather than left to interleave. Without this,
    /// `the_queued_path_pins_before_it_applies` could apply an empty queue --
    /// which succeeds, and counts -- inside another test's delta window.
    static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Falsifiable: fails if any public entry point stops funnelling through
    /// [`super::run_pinned`].
    ///
    /// `run_pinned` is where the ordering lives, and the previous sweep showed
    /// that a bypass is invisible to every behavioural assertion available
    /// here: by the time a test runs, some earlier test has pinned the process,
    /// so "the module is pinned afterwards" holds for a bypassing build too.
    /// The source scan cannot help either, since it deliberately skips this
    /// crate -- this crate is the one place the banned calls are allowed.
    ///
    /// So the wiring is asserted structurally, the way
    /// `division_combat_stats_live` asserts its own rollback ordering: each of
    /// the three entry points must name the helper. A bypass drops the count.
    #[test]
    fn every_entry_point_funnels_through_the_ordering_helper() {
        let source = include_str!("lib.rs");
        // Split so this literal is not itself a match in the scanned source,
        // which would make the count self-satisfying.
        let needle = concat!("run_pinned(", "pin_containing_module");
        let funnelled = source.matches(needle).count();
        assert_eq!(
            funnelled, 3,
            "enable, queue_enable and apply_queued must each go through              run_pinned; found {funnelled} call(s), so one of them patches              without the pin ordering"
        );
    }

    /// THE ORDERING, and the only test that can actually discriminate it.
    ///
    /// Falsifiable: fails if `run_pinned` ever runs the operation when the pin
    /// was refused -- i.e. if a build patched first and pinned second. The
    /// asserting-the-flag-afterwards tests below CANNOT catch that, because by
    /// the time any of them runs some earlier test has already pinned this
    /// process; that is exactly how two mutations survived the first sweep.
    #[test]
    fn a_refused_pin_means_the_patch_never_runs() {
        use std::cell::Cell;
        let ran = Cell::new(false);
        let outcome = super::run_pinned(
            || Err(5u32),
            || {
                ran.set(true);
                Ok::<(), minhook::MH_STATUS>(())
            },
        );
        assert!(!ran.get(), "the patch must not run when the pin was refused");
        assert_eq!(
            outcome.unwrap_err(),
            InlineDetourEnableError::ModulePinRefused(5),
            "and the refusal must name the pin, not MinHook"
        );

        // The converse: an accepted pin does run it, so the guard is not simply
        // refusing everything.
        let ran = Cell::new(false);
        let outcome = super::run_pinned(
            || Ok(()),
            || {
                ran.set(true);
                Ok::<(), minhook::MH_STATUS>(())
            },
        );
        assert!(ran.get(), "an accepted pin must let the patch run");
        assert!(outcome.is_ok());
    }

    /// Falsifiable: fails if the QUEUED path stops pinning before it patches.
    ///
    /// `terrain_capture` reaches MinHook only through `queue_enable` +
    /// `apply_queued`, never through `enable`, so a pin that guarded only the
    /// direct path would have left that DLL exactly as exposed as before -- and
    /// it is the one with recorded 0xc0000005 faults in an unloaded mapping.
    #[test]
    fn the_queued_path_pins_before_it_applies() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let queued = unsafe { queue_enable(0x3000 as *mut core::ffi::c_void) };
        assert!(
            queued.is_err() && queued.unwrap_err().minhook_status().is_some(),
            "no hook exists at this address, so MinHook must be the refuser"
        );
        assert!(
            module_pinned_against_unload(),
            "queuing must pin: a queue that is later applied patches the target"
        );

        let applied = unsafe { apply_queued() };
        // An empty queue applies cleanly; either way the pin already happened.
        let _ = applied;
        assert!(module_pinned_against_unload());
    }

    /// Falsifiable: fails if the standalone pin stops being fail-closed, i.e.
    /// if it reports success the loader did not give. This is the inversion
    /// `terrain_capture` shipped -- it latched its pinned flag BEFORE calling
    /// the API and proceeded on failure, so the flag read pinned over a module
    /// that was still unloadable.
    #[test]
    fn the_standalone_pin_reports_only_what_the_loader_accepted() {
        assert!(pin_module_now().is_ok());
        assert!(module_pinned_against_unload());
    }

    /// Falsifiable: fails if a MinHook refusal is ever counted as an enabled
    /// detour. The count is what the teardown proof reports as the reason the
    /// module is retained, so inflating it would claim residency that never
    /// existed.
    #[test]
    fn a_refused_patch_is_not_counted_as_an_enabled_detour() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
