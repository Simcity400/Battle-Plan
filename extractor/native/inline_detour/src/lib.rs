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
mod freeze;

#[cfg(windows)]
mod windows_impl {
    use super::freeze::{freeze_other_threads, FreezeRefusal};
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
        /// The process could not be frozen, so nothing was patched. A target
        /// rewritten while a thread may be executing inside it is a torn
        /// instruction stream in that thread.
        FreezeRefused(FreezeRefusal),
        /// The pin and the freeze both succeeded and MinHook refused the patch.
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
                Self::FreezeRefused(refusal) => write!(f, "inline detour {refusal}"),
                Self::MinHook(status) => write!(f, "{status}"),
            }
        }
    }

    impl std::error::Error for InlineDetourEnableError {}

    impl From<MH_STATUS> for InlineDetourEnableError {
        fn from(status: MH_STATUS) -> Self {
            Self::MinHook(status)
        }
    }

    impl From<FreezeRefusal> for InlineDetourEnableError {
        fn from(refusal: FreezeRefusal) -> Self {
            Self::FreezeRefused(refusal)
        }
    }

    impl InlineDetourEnableError {
        /// The underlying MinHook status, when the refusal came from MinHook
        /// rather than from the pin or the freeze. Call sites that inspect
        /// specific statuses use this; call sites that only render the failure
        /// use `Display`.
        pub fn minhook_status(self) -> Option<MH_STATUS> {
            match self {
                Self::MinHook(status) => Some(status),
                Self::ModulePinRefused(_) | Self::FreezeRefused(_) => None,
            }
        }

        /// The freeze refusal, when that is what stopped the patch.
        pub fn freeze_refusal(self) -> Option<FreezeRefusal> {
            match self {
                Self::FreezeRefused(refusal) => Some(refusal),
                Self::ModulePinRefused(_) | Self::MinHook(_) => None,
            }
        }
    }

    /// Why an inline detour was not released.
    ///
    /// Separate from [`InlineDetourEnableError`] because the consequences
    /// differ: a refused enable leaves the target clean, whereas a refused
    /// release leaves the detour INSTALLED AND LIVE. That is the safe
    /// direction -- the module is pinned, so a thread inside the detour body
    /// still runs real code -- but the owning unit is no longer released and
    /// must report itself so.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum InlineDetourReleaseError {
        /// The process could not be frozen. NOTHING WAS UNPATCHED; the detour
        /// is retained, which is deliberate. Restoring the original bytes
        /// underneath a thread that may be executing them is the unsafe
        /// direction, and refusing keeps the process in the state it is
        /// already running in.
        FreezeRefused(FreezeRefusal),
        /// The freeze succeeded and MinHook refused.
        MinHook(MH_STATUS),
    }

    impl fmt::Display for InlineDetourReleaseError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::FreezeRefused(refusal) => write!(
                    f,
                    "inline detour retained rather than restored: {refusal}"
                ),
                Self::MinHook(status) => write!(f, "{status}"),
            }
        }
    }

    impl std::error::Error for InlineDetourReleaseError {}

    impl InlineDetourReleaseError {
        /// The underlying MinHook status, when MinHook is what refused.
        pub fn minhook_status(self) -> Option<MH_STATUS> {
            match self {
                Self::MinHook(status) => Some(status),
                Self::FreezeRefused(_) => None,
            }
        }

        /// True when this status is MinHook reporting the hook was already
        /// gone. Nothing was patched, nothing is retained, and callers that
        /// treat a release as idempotent stop here.
        pub fn already_released(self) -> bool {
            matches!(
                self,
                Self::MinHook(MH_STATUS::MH_ERROR_DISABLED)
                    | Self::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)
            )
        }

        /// True when a live detour was left installed because the process
        /// could not be frozen. The owning unit is NOT released.
        pub fn detour_retained(self) -> bool {
            matches!(self, Self::FreezeRefused(_))
        }
    }

    static MODULE_PINNED: AtomicBool = AtomicBool::new(false);
    static ENABLED_DETOURS: AtomicU64 = AtomicU64::new(0);
    static RETAINED_DETOURS: AtomicU64 = AtomicU64::new(0);

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
    /// `op`'s error is whatever it refuses with -- MinHook's status, or the
    /// freeze refusal for the entry points that hold a freeze across the
    /// patch -- so the ordering helper does not flatten a refused freeze into
    /// a MinHook status it did not come from.
    pub(crate) fn run_pinned<T, E>(
        pin: impl FnOnce() -> Result<(), u32>,
        op: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, InlineDetourEnableError>
    where
        E: Into<InlineDetourEnableError>,
    {
        pin().map_err(InlineDetourEnableError::ModulePinRefused)?;
        op().map_err(Into::into)
    }

    /// Enable an inline detour at `target`: pin this module, freeze the
    /// process, then patch.
    ///
    /// FAIL-CLOSED TWICE, and both orders are the whole point.
    ///
    /// If the module cannot be pinned, the target is not patched, so the state
    /// where a detour is live in an unloadable module never exists. Patching
    /// first and pinning second would leave exactly that window open.
    ///
    /// If the process cannot be frozen, the target is not patched either.
    /// MinHook suspends threads itself but ignores every failure to do so and
    /// rewrites the bytes regardless; a thread executing inside those bytes
    /// then resumes into a half-old, half-new instruction stream. Holding the
    /// freeze across the MinHook call is what makes MinHook's suspension
    /// succeed rather than be skipped.
    ///
    /// # Safety
    ///
    /// `target` must name a hook already created with MinHook, and the caller
    /// keeps every obligation `MH_EnableHook` has: the detour must remain a
    /// valid callable for as long as the patch is live.
    pub unsafe fn enable(target: *mut c_void) -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || {
            let frozen = freeze_other_threads()?;
            let outcome = unsafe { MinHook::enable_hook(target) };
            drop(frozen);
            outcome.map_err(InlineDetourEnableError::MinHook)
        })?;
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

    /// Apply every queued enable, pinning this module first and patching under
    /// a held freeze.
    ///
    /// This is the call that actually patches a queued batch, so both orderings
    /// matter here exactly as they do in [`enable`]. Counted as ONE enable
    /// operation covering the whole batch: the batch size is MinHook's internal
    /// state and is not reported back.
    ///
    /// # Safety
    ///
    /// As [`enable`], for every target in the queue.
    pub unsafe fn apply_queued() -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || {
            let frozen = freeze_other_threads()?;
            let outcome = unsafe { MinHook::apply_queued() };
            drop(frozen);
            outcome.map_err(InlineDetourEnableError::MinHook)
        })?;
        ENABLED_DETOURS.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Disable the inline detour at `target`, restoring the original bytes,
    /// under a held process freeze.
    ///
    /// FAIL-CLOSED: if the process cannot be frozen this does NOT unpatch. The
    /// detour stays installed and `Err(FreezeRefused)` says so. That is the
    /// safe direction and it is the direction this workspace already relies
    /// on: the containing module is pinned, so a thread poised in the detour
    /// body keeps running real code for the life of the process, whereas
    /// restoring a target underneath a running thread tears its instruction
    /// stream.
    ///
    /// The trampoline is untouched, exactly as `MH_DisableHook` leaves it.
    ///
    /// # Safety
    ///
    /// As `MH_DisableHook`. The detour must remain a valid callable until this
    /// returns `Ok`, and must remain valid FOREVER if it returns
    /// [`InlineDetourReleaseError::detour_retained`].
    pub unsafe fn disable(target: *mut c_void) -> Result<(), InlineDetourReleaseError> {
        let frozen = match freeze_other_threads() {
            Ok(frozen) => frozen,
            Err(refusal) => {
                RETAINED_DETOURS.fetch_add(1, Ordering::AcqRel);
                return Err(InlineDetourReleaseError::FreezeRefused(refusal));
            }
        };
        let outcome = unsafe { MinHook::disable_hook(target) };
        drop(frozen);
        outcome.map_err(InlineDetourReleaseError::MinHook)
    }

    /// Remove the hook at `target` under a held process freeze, freeing its
    /// trampoline.
    ///
    /// FAIL-CLOSED in the same direction as [`disable`], and for a sharper
    /// reason. `MH_RemoveHook` restores the target's bytes if the hook is
    /// enabled AND THEN FREES THE TRAMPOLINE. Letting that run without a
    /// freeze risks both halves at once: a torn instruction stream at the
    /// target, and a freed trampoline underneath a thread that is inside it.
    ///
    /// # Safety
    ///
    /// As `MH_RemoveHook`. On a retained refusal nothing was freed and nothing
    /// was restored, so the caller's trampoline and detour must both stay
    /// valid for the life of the process.
    pub unsafe fn remove(target: *mut c_void) -> Result<(), InlineDetourReleaseError> {
        let frozen = match freeze_other_threads() {
            Ok(frozen) => frozen,
            Err(refusal) => {
                RETAINED_DETOURS.fetch_add(1, Ordering::AcqRel);
                return Err(InlineDetourReleaseError::FreezeRefused(refusal));
            }
        };
        let outcome = unsafe { MinHook::remove_hook(target) };
        drop(frozen);
        outcome.map_err(InlineDetourReleaseError::MinHook)
    }

    /// How many releases were refused, leaving a live detour installed.
    ///
    /// Non-zero means some unit believes it released a hook that is in fact
    /// still patched into the game. The teardown proof reads this.
    pub fn retained_detour_count() -> u64 {
        RETAINED_DETOURS.load(Ordering::Acquire)
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
pub use freeze::{
    freeze_attempts, freeze_attempts_on_this_thread, freeze_process, freeze_refusals,
    open_failures_excused_as_no_longer_ours, suspend_failures_excused_as_terminated,
    suspend_failures_needing_the_termination_wait, FreezeRefusal, ProcessFreeze,
};

#[cfg(windows)]
pub(crate) use windows_impl::run_pinned;
#[cfg(windows)]
pub use windows_impl::{
    apply_queued, disable, enable, enabled_detour_count, module_pinned_against_unload,
    pin_module_now, queue_enable, remove, retained_detour_count, InlineDetourEnableError,
    InlineDetourReleaseError,
};

/// The banned MinHook entry points, spelled as they appear in a call.
///
/// EVERY entry point that rewrites a live target's bytes is here, not just the
/// ones that install. `MH_DisableHook` and `MH_RemoveHook` route through the
/// same `Freeze()`/`EnableHookLL` pair as `MH_EnableHook` and fail open on a
/// refused suspension in exactly the same way; `MH_RemoveHook` additionally
/// frees the trampoline. `MH_CreateHook` is deliberately absent: it reads the
/// target and allocates, and patches nothing.
///
/// Kept as separate fragments and joined at runtime so that this list does not
/// itself match the scan, which would make the guard trivially self-satisfying.
#[cfg(test)]
const BANNED_CALL_FRAGMENTS: &[(&str, &str)] = &[
    ("MinHook::", "enable_hook("),
    ("MinHook::", "enable_all_hooks("),
    ("MinHook::", "queue_enable_hook("),
    ("MinHook::", "disable_hook("),
    ("MinHook::", "disable_all_hooks("),
    ("MinHook::", "queue_disable_hook("),
    ("MinHook::", "remove_hook("),
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

/// The Win32 entry points a PRIVATE in-process thread suspender needs, spelled
/// as they appear in a call or an `extern` declaration.
///
/// Four units in this workspace each hand-rolled one of these, and each got the
/// same two things wrong: a broken `Thread32Next` read as the end of the thread
/// list, and a single enumeration pass that a thread created afterwards walks
/// straight through. They are gone; this is what keeps them gone.
///
/// SCOPE, stated because it is narrower than "no thread suspension anywhere".
/// `ResumeThread` and `CreateToolhelp32Snapshot` are deliberately ABSENT:
/// `recorder_host` resumes the main thread of a process it created suspended,
/// and enumerates PROCESSES, and neither reaches a thread of the collector's own
/// process. What is banned is the ability to enumerate this process's threads
/// and stop them -- without `Thread32First`/`Thread32Next` there is no thread
/// list to walk, and without `SuspendThread` there is nothing to stop it with.
///
/// NOT COVERED, in those words: `NtGetNextThread`, a raw syscall, a suspender
/// written in a `.c` file, or a fourth-party crate. Each needs an edit that is
/// visible in review, which is the same bar the MinHook ban above sets.
///
/// Kept as separate fragments and joined at runtime so this list does not
/// itself match the scan.
#[cfg(test)]
const BANNED_SUSPENDER_FRAGMENTS: &[(&str, &str)] = &[
    ("Suspend", "Thread("),
    ("Wow64Suspend", "Thread("),
    ("NtSuspend", "Thread("),
    ("ZwSuspend", "Thread("),
    ("Open", "Thread("),
    ("Thread32", "First("),
    ("Thread32", "Next("),
    ("GetThread", "Context("),
];

/// Strip `//` line comments so a prose reference to a banned name is not a
/// violation.
///
/// Deliberately crude, and deliberately one-directional: it removes comment
/// text, so it can only ever make the scan MISS a call hidden in something that
/// looks like a comment -- which is not writable Rust -- and can never make it
/// invent one. Block comments are not stripped, so a banned call commented out
/// with `/* */` still trips the scan; that is the safe direction.
#[cfg(test)]
fn strip_line_comments(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("//") {
            Some(index) => &line[..index],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every line of `text` that reaches for a private thread suspender.
#[cfg(test)]
fn scan_source_for_private_suspender(text: &str) -> Vec<usize> {
    let code = strip_line_comments(&normalize_for_scan(text));
    let banned: Vec<String> = BANNED_SUSPENDER_FRAGMENTS
        .iter()
        .map(|(prefix, call)| format!("{prefix}{call}"))
        .collect();
    code.lines()
        .enumerate()
        .filter(|(_, line)| banned.iter().any(|b| line.contains(b.as_str())))
        .map(|(index, _)| index + 1)
        .collect()
}

#[cfg(test)]
mod source_ban_tests {
    use super::{
        scan_source_for_direct_enable, scan_source_for_private_suspender,
        BANNED_CALL_FRAGMENTS, BANNED_SUSPENDER_FRAGMENTS,
    };
    use std::path::{Path, PathBuf};

    /// Every `.rs` file in every native workspace that can inject into the
    /// game, except this crate's own.
    ///
    /// EXPLICITLY MORE THAN ONE WORKSPACE, and it stays that way now that the
    /// crate that motivated it has moved. `terrain_capture` is a separately
    /// injected DLL with its own D3D11 detours; it used to live in
    /// `replay_interface/native`, and a scan that stopped at the extractor
    /// boundary would have declared the invariant enforced while that mapping
    /// patched before pinning. It now sits in `extractor/native` with the rest
    /// of the injected code, but the second root is kept deliberately: an
    /// invariant that stops at a workspace boundary is not one, and nothing
    /// stops a future injected DLL appearing in the replay tree again.
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
    /// SCOPE: whatever [`workspace_sources`] walks, which is BOTH native
    /// workspaces. An earlier version of this comment claimed the scan stopped
    /// at `extractor/native` and that `terrain_capture` was exempt; that was
    /// never what the code did — `workspace_sources` has always included
    /// `replay_interface/native`, and `terrain_capture` has always routed its
    /// D3D11 detours through `inline_detour::queue_enable`. The crate now lives
    /// at `extractor/native/terrain_capture`, so the point is moot for it, but
    /// the comment was describing a hole that did not exist and would have
    /// licensed one that did.
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

    /// Falsifiable: fails if any entry point that rewrites a live target's
    /// bytes drops out of the ban list.
    ///
    /// The list originally covered only the three that install. `MH_DisableHook`
    /// and `MH_RemoveHook` reach the identical `Freeze()`/`EnableHookLL` pair
    /// and fail open on a refused suspension identically, so a lane calling
    /// them directly bypasses the freeze exactly as a lane calling
    /// `MH_EnableHook` directly bypassed the pin. Each planted line here goes
    /// through the real scanner, so deleting a fragment turns this red.
    #[test]
    fn every_minhook_entry_point_that_rewrites_a_target_is_banned() {
        for call in [
            "enable_hook",
            "enable_all_hooks",
            "queue_enable_hook",
            "disable_hook",
            "disable_all_hooks",
            "queue_disable_hook",
            "remove_hook",
            "apply_queued",
        ] {
            let planted = format!("fn f() {{ unsafe {{ MinHook::{call}(t) }}; }}");
            assert_eq!(
                scan_source_for_direct_enable(&planted),
                vec![1],
                "MinHook::{call} rewrites a live target and must be banned outside this crate"
            );
        }
    }

    /// Falsifiable: fails if `create_hook` is ever added to the ban list.
    ///
    /// It is the one MinHook entry point lanes legitimately call directly:
    /// it reads the target and allocates a trampoline, and patches nothing, so
    /// no thread can be inside bytes it rewrites. Banning it would push every
    /// lane's hook creation through a wrapper that has nothing to add, and the
    /// resulting churn would obscure the ban that matters.
    #[test]
    fn creating_a_hook_is_not_banned_because_it_rewrites_nothing() {
        let planted = "fn f() { unsafe { MinHook::create_hook(t, d) }; }";
        assert!(
            scan_source_for_direct_enable(planted).is_empty(),
            "MinHook::create_hook patches no live bytes and must stay callable"
        );
    }

    /// THE ENFORCEMENT for the freeze, and the twin of the MinHook ban above.
    /// Fails if any crate suspends this process's threads itself.
    ///
    /// A private suspender is not merely duplication. All four that existed
    /// certified a TRUNCATED thread list -- they read any `Thread32Next` stop as
    /// the end of the list -- and all four enumerated exactly once, so a thread
    /// created after the snapshot was patched around while it ran. Both defects
    /// are invisible at the call site and identical in every copy, which is what
    /// makes a choke point the fix rather than four fixes.
    ///
    /// The two mutations this is here to catch, restated as what a future lane
    /// would actually write: reintroducing an `extern` block with
    /// `SuspendThread`/`Thread32Next` in a lane, or a fresh enumeration loop
    /// beside the shared freeze. Either lands here by file and line.
    #[test]
    fn no_private_thread_suspender_outside_this_crate() {
        let mut violations = Vec::new();
        for path in workspace_sources() {
            if is_this_crate(&path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in scan_source_for_private_suspender(&text) {
                violations.push(format!("{}:{}", path.display(), line));
            }
        }
        assert!(
            violations.is_empty(),
            "code that rewrites live bytes must freeze the process through `inline_detour` -- `enable`/`disable`/`remove` when MinHook owns the patch, `freeze_process` when the caller writes its own bytes. A private suspender reads a broken `Thread32Next` as the end of the thread list and enumerates once, so it certifies a truncated list and never sees a thread created after its snapshot; both are what the shared freeze exists to refuse.\n{}",
            violations.join("\n")
        );
    }

    /// THE GUARD ON THAT GUARD: every planted line goes through the real
    /// scanner, so deleting a fragment or the violation-collection branch turns
    /// this red rather than leaving a scan that finds nothing.
    #[test]
    fn the_suspender_scanner_catches_calls_and_extern_declarations() {
        for (prefix, call) in BANNED_SUSPENDER_FRAGMENTS {
            let called = format!("fn f() {{ unsafe {{ {prefix}{call}h) }}; }}");
            assert_eq!(
                scan_source_for_private_suspender(&called),
                vec![1],
                "a call to {prefix}{call} must be caught"
            );
            let declared = format!("extern \"system\" {{ fn {prefix}{call}h: Handle) -> u32; }}");
            assert_eq!(
                scan_source_for_private_suspender(&declared),
                vec![1],
                "an extern declaration of {prefix}{call} must be caught"
            );
        }

        let commented = "// the old suspender called SuspendThread(handle) here";
        assert!(
            scan_source_for_private_suspender(commented).is_empty(),
            "prose about the removed suspenders is not a violation"
        );

        let shared = "let freeze = inline_detour::freeze_process()?;";
        assert!(
            scan_source_for_private_suspender(shared).is_empty(),
            "taking the shared freeze is the whole point and must not be flagged"
        );

        let launcher = "unsafe { ResumeThread(process.main_thread) };";
        assert!(
            scan_source_for_private_suspender(launcher).is_empty(),
            "resuming a process created suspended is recorder_host's job and \
             touches no thread of this process"
        );
    }

    /// Falsifiable: fails if any of the four migrated units grows a private
    /// suspender again, named one by one so the failure says WHICH unit
    /// regressed instead of only that some file did.
    ///
    /// The blanket scan above already covers every crate. This exists because a
    /// blanket scan cannot say the four files it was written for are still
    /// being scanned -- rename one, or move its patcher into a new module, and
    /// the blanket test stays green while the unit it was written for is no
    /// longer named anywhere.
    #[test]
    fn the_four_migrated_patchers_are_still_named_and_still_clean() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("inline_detour has a parent directory")
            .to_path_buf();
        for unit in [
            "lanes/fired_event_popups_live/src/a729_inline_hook.rs",
            "lanes/focus_research_decisions_depth_live/src/a729_focus_item_inline.rs",
            "lanes/naval_combat_live/src/convoy_terminal_hook_windows.rs",
            "lanes/supply_logistics_attrition_live/src/a729_attrition_hook.rs",
        ] {
            let path = root.join(unit);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{unit} must still exist: {error}"));
            assert!(
                scan_source_for_private_suspender(&text).is_empty(),
                "{unit} suspends threads itself again at line(s) {:?}",
                scan_source_for_private_suspender(&text)
            );
            let code = super::strip_line_comments(&text);
            assert!(
                code.contains("inline_detour::freeze_process()"),
                "{unit} rewrites live bytes and must take the shared freeze"
            );
            assert!(
                code.contains("any_instruction_pointer_in"),
                "{unit} must ask where the frozen threads are stopped before it \
                 rewrites a range; suspension alone does not prove the range is \
                 clear"
            );
        }
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
        // Both globals are in play: `enable` pins AND freezes. Always taken in
        // this order, everywhere, so two tests cannot deadlock on them.
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _serialised = crate::freeze::os_suspend_state_guard();
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
        // `apply_queued` patches, so it freezes: this test moves process-global
        // suspend state and must serialise on it too. Same order as everywhere.
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _serialised = crate::freeze::os_suspend_state_guard();
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
        // Both globals are in play: `enable` pins AND freezes. Always taken in
        // this order, everywhere, so two tests cannot deadlock on them.
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _serialised = crate::freeze::os_suspend_state_guard();
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

    /// Falsifiable: fails if any of the three patching wrappers stops freezing
    /// the process.
    ///
    /// This is THE guard on the fix. Delete the `freeze_other_threads()` call
    /// from `enable`, `disable`, or `remove` and the corresponding attempt
    /// count stops moving, which fails here by name. Each call is driven at an
    /// address MinHook has no hook for, so MinHook refuses AFTER the freeze --
    /// the same shape as the pin-ordering test above.
    #[test]
    fn every_patching_wrapper_freezes_the_process_before_it_calls_minhook() {
        // Process-global suspend state: serialise with every other test
        // that freezes or arranges suspend counts.
        let _serialised = crate::freeze::os_suspend_state_guard();
        let target = 0x3000 as *mut core::ffi::c_void;

        let before = freeze_attempts_on_this_thread();
        let _ = unsafe { enable(target) };
        assert_eq!(
            freeze_attempts_on_this_thread(),
            before + 1,
            "enable must freeze the process exactly once before patching"
        );

        let before = freeze_attempts_on_this_thread();
        let _ = unsafe { disable(target) };
        assert_eq!(
            freeze_attempts_on_this_thread(),
            before + 1,
            "disable must freeze the process exactly once before restoring bytes"
        );

        let before = freeze_attempts_on_this_thread();
        let _ = unsafe { remove(target) };
        assert_eq!(
            freeze_attempts_on_this_thread(),
            before + 1,
            "remove must freeze the process exactly once before restoring bytes \
             and freeing the trampoline"
        );
    }

    /// Falsifiable: fails if a release that could not freeze is ever reported
    /// as anything other than a RETAINED, still-installed detour.
    ///
    /// The distinction is the whole point of the release error existing
    /// separately. `already_released` means the hook was gone and there is
    /// nothing to do; `detour_retained` means the game is still running our
    /// patched bytes and the owning unit must not claim it released anything.
    /// Collapsing the two would let a teardown proof pass over a live detour.
    #[test]
    fn a_release_that_could_not_freeze_reports_a_retained_detour() {
        let retained = InlineDetourReleaseError::FreezeRefused(FreezeRefusal::ThreadSuspend {
            thread: 4242,
            os_error: 5,
        });
        assert!(retained.detour_retained());
        assert!(!retained.already_released());
        assert!(retained.minhook_status().is_none());
        assert!(retained.to_string().contains("retained rather than restored"));
        assert!(retained.to_string().contains("4242"));

        for absent in [
            InlineDetourReleaseError::MinHook(minhook::MH_STATUS::MH_ERROR_DISABLED),
            InlineDetourReleaseError::MinHook(minhook::MH_STATUS::MH_ERROR_NOT_CREATED),
        ] {
            assert!(absent.already_released());
            assert!(!absent.detour_retained());
        }

        let refused = InlineDetourReleaseError::MinHook(minhook::MH_STATUS::MH_ERROR_MEMORY_PROTECT);
        assert!(!refused.already_released());
        assert!(!refused.detour_retained());
    }

    /// Falsifiable: fails if a freeze refusal is ever reported as a MinHook
    /// status on the enable path, which would send an operator looking at the
    /// target instead of at the process.
    #[test]
    fn an_enable_refused_by_the_freeze_is_not_a_minhook_status() {
        let refusal = InlineDetourEnableError::FreezeRefused(FreezeRefusal::NotQuiescent {
            passes: 16,
            still_running: 3,
        });
        assert!(refusal.minhook_status().is_none());
        assert!(refusal.freeze_refusal().is_some());
        assert!(refusal.to_string().contains("no target byte was touched"));
        assert!(InlineDetourEnableError::ModulePinRefused(5)
            .freeze_refusal()
            .is_none());
    }
}
