//! Exact-a729 country-trade window activation observer.
//!
//! `CCountryTradeView::Open` and `CCountryTradeView::Close` are the engine
//! lifecycle boundary for the authored `countrytradeview` root.  The hook
//! publishes only a pointer-free-able country object identity plus a monotonic
//! lifecycle sequence.  The d43 collector must join that object to its exact
//! current country census before emitting MTRA; a missing join is omission,
//! never an inferred selected POV.

use core::ffi::c_void;
use minhook::MinHook;
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const COUNTRY_TRADE_OPEN_RVA: usize = 0x015e_c520;
pub const COUNTRY_TRADE_CLOSE_RVA: usize = 0x015e_af40;
pub const COUNTRY_FROM_REFERENCE_RVA: usize = 0x00ba_56a0;
pub const COUNTRY_TRADE_VIEW_VTABLE_RVA: usize = 0x029c_b560;
pub const COUNTRY_SOURCE_HOLDER_OFFSET: usize = 0x548;
pub const COUNTRY_REFERENCE_VIRTUAL_OFFSET: usize = 0xa0;

pub const AUTHORITY_JOB_ID: &str =
    "20260722T013354Z-x15-open-close-lifecycle-521d1851";
pub const AUTHORITY_MANIFEST_SHA256: &str =
    "de07756d379ffa51de678e5083ac8f4c6efb7453bfc20306499c44b18fba78e2";
pub const SENTINEL_JOB_ID: &str =
    "20260722T013608Z-x15-activation-hook-sentinels-feaea801";
pub const SENTINEL_ARTIFACT_SHA256: &str =
    "7dff1c3afb8d48be2987c5ce1e9f9ade92cf805094ef634fef75b444d835c62b";

const OPEN_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xe8, 0x82, 0x2b, 0x81, 0xff, 0x83,
    0x38,
];
const CLOSE_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8b, 0x89, 0x10, 0x1f, 0x00,
    0x00,
];

type WindowLifecycleFn = unsafe extern "system" fn(usize);
type CountryReferenceFn = unsafe extern "system" fn(usize) -> usize;
type CountryFromReferenceFn = unsafe extern "system" fn(usize) -> usize;

static ORIGINAL_OPEN: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CLOSE: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static BROKEN: AtomicBool = AtomicBool::new(false);
static DRAINING: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static PUBLISH_EPOCH: AtomicU64 = AtomicU64::new(0);
static ACTIVE_VIEW: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_COUNTRY: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActivationSnapshot {
    pub active: bool,
    pub country_ptr: usize,
    pub sequence: u64,
}

struct CallbackGuard;

impl CallbackGuard {
    fn enter() -> Self {
        ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel);
        Self
    }

    fn captures(&self) -> bool {
        !DRAINING.load(Ordering::Acquire)
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel);
    }
}

unsafe extern "system" fn hook_open(view: usize) {
    let guard = CallbackGuard::enter();
    let original: WindowLifecycleFn =
        unsafe { mem::transmute(ORIGINAL_OPEN.load(Ordering::Acquire)) };
    unsafe { original(view) };
    if !guard.captures() {
        return;
    }
    let Some(country_ptr) = exact_selected_country(view) else {
        // An engine-open window without an exact country join is not active
        // authority.  Clear any previous observation instead of retaining it.
        publish(0, 0, next_sequence());
        return;
    };
    publish(view, country_ptr, next_sequence());
}

unsafe extern "system" fn hook_close(view: usize) {
    let guard = CallbackGuard::enter();
    if guard.captures() {
        publish(0, 0, next_sequence());
    }
    let original: WindowLifecycleFn =
        unsafe { mem::transmute(ORIGINAL_CLOSE.load(Ordering::Acquire)) };
    unsafe { original(view) };
}

fn next_sequence() -> u64 {
    NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel).max(1)
}

fn publish(view: usize, country_ptr: usize, sequence: u64) {
    PUBLISH_EPOCH.fetch_add(1, Ordering::AcqRel);
    ACTIVE_VIEW.store(view, Ordering::Relaxed);
    ACTIVE_COUNTRY.store(country_ptr, Ordering::Relaxed);
    ACTIVE_SEQUENCE.store(sequence, Ordering::Relaxed);
    PUBLISH_EPOCH.fetch_add(1, Ordering::Release);
}

fn exact_selected_country(view: usize) -> Option<usize> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0
        || read_value::<usize>(view)? != base.checked_add(COUNTRY_TRADE_VIEW_VTABLE_RVA)?
    {
        return None;
    }
    let holder = read_value::<usize>(view.checked_add(COUNTRY_SOURCE_HOLDER_OFFSET)?)?;
    let vtable = read_value::<usize>(holder)?;
    let method = read_value::<usize>(vtable.checked_add(COUNTRY_REFERENCE_VIRTUAL_OFFSET)?)?;
    if method < base || method >= base.checked_add(0x4000_0000)? {
        return None;
    }
    let reference_fn: CountryReferenceFn = unsafe { mem::transmute(method) };
    let reference = unsafe { reference_fn(holder) };
    if reference == 0 {
        return None;
    }
    let resolver: CountryFromReferenceFn =
        unsafe { mem::transmute(base.checked_add(COUNTRY_FROM_REFERENCE_RVA)?) };
    let country = unsafe { resolver(reference) };
    (country != 0).then_some(country)
}

pub(crate) fn snapshot() -> Option<ActivationSnapshot> {
    if !INSTALLED.load(Ordering::Acquire) || DRAINING.load(Ordering::Acquire) {
        return None;
    }
    for _ in 0..8 {
        let before = PUBLISH_EPOCH.load(Ordering::Acquire);
        if before & 1 != 0 {
            thread::yield_now();
            continue;
        }
        let view = ACTIVE_VIEW.load(Ordering::Relaxed);
        let country_ptr = ACTIVE_COUNTRY.load(Ordering::Relaxed);
        let sequence = ACTIVE_SEQUENCE.load(Ordering::Relaxed);
        let after = PUBLISH_EPOCH.load(Ordering::Acquire);
        if before == after && after & 1 == 0 {
            return Some(ActivationSnapshot {
                active: view != 0 && country_ptr != 0,
                country_ptr,
                sequence,
            });
        }
    }
    None
}

pub(crate) fn install(module_base: usize) -> Result<(), String> {
    let _guard = INSTALL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "X15 activation hook install lock poisoned".to_string())?;
    if BROKEN.load(Ordering::Acquire) {
        return Err("X15 activation hook lifecycle is broken; process restart required".into());
    }
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    if module_base == 0 {
        return Err("X15 activation hook received a zero module base".into());
    }
    let specs = hook_specs(module_base);
    verify_sentinel("open", specs[0].0, &OPEN_SENTINEL)?;
    verify_sentinel("close", specs[1].0, &CLOSE_SENTINEL)?;
    MODULE_BASE.store(module_base, Ordering::Release);
    DRAINING.store(false, Ordering::Release);
    publish(0, 0, 0);
    let mut created = 0usize;
    for &(target, detour, original) in &specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(trampoline) => {
                original.store(trampoline as usize, Ordering::Release);
                created += 1;
            }
            Err(status) => {
                let rollback = rollback(&specs, created, 0);
                return Err(format!(
                    "X15 activation hook creation failed at 0x{target:X}: {status}{rollback}"
                ));
            }
        }
    }
    for (enabled, (target, _, _)) in specs.iter().enumerate() {
        if let Err(status) = unsafe { inline_detour::enable(*target as *mut c_void) } {
            let rollback = rollback(&specs, created, enabled);
            return Err(format!(
                "X15 activation hook enable failed at 0x{target:X}: {status}{rollback}"
            ));
        }
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub(crate) fn uninstall() -> Result<(), String> {
    let _guard = INSTALL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "X15 activation hook install lock poisoned".to_string())?;
    if BROKEN.load(Ordering::Acquire) {
        return Err("X15 activation hook lifecycle is broken; process restart required".into());
    }
    if !INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    DRAINING.store(true, Ordering::Release);
    let specs = hook_specs(MODULE_BASE.load(Ordering::Acquire));
    let mut failures = Vec::new();
    for (target, _, _) in specs.iter().rev() {
        if let Err(status) = unsafe { inline_detour::disable(*target as *mut c_void) } {
            failures.push(format!("0x{target:X}: {status}"));
        }
    }
    if !failures.is_empty() {
        BROKEN.store(true, Ordering::Release);
        return Err(format!(
            "X15 activation hook disable incomplete at {}; process restart required",
            failures.join(", ")
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            BROKEN.store(true, Ordering::Release);
            return Err("X15 activation callbacks did not quiesce; process restart required".into());
        }
        thread::yield_now();
    }
    for (target, _, original) in specs.iter().rev() {
        match unsafe { inline_detour::remove(*target as *mut c_void) } {
            Ok(()) => original.store(0, Ordering::Release),
            Err(status) => failures.push(format!("0x{target:X}: {status}")),
        }
    }
    if !failures.is_empty() {
        BROKEN.store(true, Ordering::Release);
        return Err(format!(
            "X15 activation hook removal incomplete at {}; process restart required",
            failures.join(", ")
        ));
    }
    publish(0, 0, 0);
    MODULE_BASE.store(0, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    DRAINING.store(false, Ordering::Release);
    Ok(())
}

type HookSpec = (usize, *mut c_void, &'static AtomicUsize);

fn hook_specs(base: usize) -> [HookSpec; 2] {
    [
        (
            base + COUNTRY_TRADE_OPEN_RVA,
            hook_open as *mut c_void,
            &ORIGINAL_OPEN,
        ),
        (
            base + COUNTRY_TRADE_CLOSE_RVA,
            hook_close as *mut c_void,
            &ORIGINAL_CLOSE,
        ),
    ]
}

fn rollback(specs: &[HookSpec], created: usize, enabled: usize) -> String {
    DRAINING.store(true, Ordering::Release);
    let mut failures = Vec::new();
    for (target, _, _) in specs.iter().take(enabled).rev() {
        if let Err(status) = unsafe { inline_detour::disable(*target as *mut c_void) } {
            failures.push(format!("0x{target:X} disable: {status}"));
        }
    }
    // QUIESCED: `DRAINING` closed above, every enabled target disabled, and the
    // in-flight count observed at zero here -- the same three steps `uninstall`
    // takes, and for the same reason.
    //
    // THIS IS NOT THE VACUOUS CREATE-ONLY CASE, and the parameter list is the
    // proof: `enabled` is how many of these targets were ENABLED, so whenever
    // it is non-zero a game thread can be inside a detour body right now, and
    // the trampoline the loop below frees is what its call through the original
    // returns into. The freeze inside `remove` cannot see that thread -- the
    // body is this module's own function, at an address `inline_detour` is
    // never told. Classifying this path as create-only because it is named
    // `rollback` is what a name-based audit does.
    let deadline = Instant::now() + Duration::from_secs(5);
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        if Instant::now() >= deadline {
            // RETAINED, not freed: the targets are disabled so nothing new can
            // enter them, and every trampoline stays allocated for whoever is
            // still inside. `BROKEN` is the same latch a non-quiescing
            // `uninstall` sets.
            BROKEN.store(true, Ordering::Release);
            return "; rollback incomplete: callbacks did not quiesce, hooks retained".to_string();
        }
        thread::yield_now();
    }
    for (target, _, original) in specs.iter().take(created).rev() {
        match unsafe { inline_detour::remove(*target as *mut c_void) } {
            Ok(()) => original.store(0, Ordering::Release),
            Err(status) => failures.push(format!("0x{target:X} remove: {status}")),
        }
    }
    MODULE_BASE.store(0, Ordering::Release);
    if failures.is_empty() {
        DRAINING.store(false, Ordering::Release);
        String::new()
    } else {
        BROKEN.store(true, Ordering::Release);
        format!("; rollback incomplete at {}", failures.join(", "))
    }
}

fn verify_sentinel(label: &str, address: usize, expected: &[u8]) -> Result<(), String> {
    let mut actual = vec![0u8; expected.len()];
    if !read_bytes(address, &mut actual) || actual != expected {
        return Err(format!(
            "X15 activation {label} sentinel mismatch at 0x{address:X}"
        ));
    }
    Ok(())
}

fn read_value<T: Copy>(address: usize) -> Option<T> {
    let mut value = mem::MaybeUninit::<T>::uninit();
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            value.as_mut_ptr().cast(),
            mem::size_of::<T>(),
            &mut read,
        )
    } != 0;
    (ok && read == mem::size_of::<T>()).then(|| unsafe { value.assume_init() })
}

fn read_bytes(address: usize, out: &mut [u8]) -> bool {
    let mut read = 0usize;
    (unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            out.as_mut_ptr().cast(),
            out.len(),
            &mut read,
        )
    }) != 0
        && read == out.len()
}
