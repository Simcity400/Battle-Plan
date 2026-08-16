//! Sentinel-gated exact-a729 producer/formatter pairing for province supply.

use core::ffi::c_void;
use inline_detour::InlineDetourReleaseError;
use minhook::{MinHook, MH_STATUS};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, RtlCaptureStackBackTrace};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::{
    ExactProvinceSupplyTooltipBatch, ExactProvinceSupplyTooltipRow, DISPLAY_DECIMALS,
    FIXED_SCALE, MAX_OBSERVER_TAG_DB_ID, MAX_PROVINCE_ID, MAX_ROWS,
};

pub const PRODUCER_RVA: u32 = 0x0162_3070;
pub const FORMATTER_RVA: u32 = 0x024B_6200;
pub const FORMATTER_CALL_RVA: u32 = 0x0162_38F4;
pub const FORMATTER_RETURN_RVA: u32 = 0x0162_38F9;
const CURRENT_COUNTRY_REF_RVA: usize = 0x0162_73C0;
const COUNTRY_REF_TO_INDEX_RVA: usize = 0x00BA_6240;
const CURRENT_TICK_RVA: usize = 0x0331_BFE0;
const CPROVINCE_ID_OFFSET: usize = 0xA4;

const PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x50, 0x10, 0x48, 0x89, 0x48, 0x08, 0x55, 0x53, 0x56, 0x57,
    0x41,
];
const FORMATTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x53, 0x48, 0x83, 0xec, 0x40,
    0x45,
];
const CALLSITE_SENTINEL: [u8; 21] = [
    0x41, 0xb8, 0x02, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xd3, 0x48, 0x8d, 0x8d, 0xd8, 0x01, 0x00,
    0x00, 0xe8, 0x07, 0x29, 0xe9, 0x00,
];

type ProducerFn = unsafe extern "system" fn(usize, usize);
type FormatterFn = unsafe extern "system" fn(usize, i64, u32) -> usize;
type CurrentCountryRefFn = unsafe extern "system" fn() -> *const i32;
type CountryRefToIndexFn = unsafe extern "system" fn(*const i32) -> u32;

static ORIGINAL_PRODUCER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_FORMATTER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ACTIVE_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
/// Are the detours allowed to do their work?
///
/// Closed BEFORE any teardown drains, so the drain measures a set that is
/// emptying rather than one that keeps refilling. A detour with admission closed
/// still calls the original -- it must, the game is waiting on it -- and does
/// nothing else.
static ADMISSION_OPEN: AtomicBool = AtomicBool::new(false);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static NEXT_HOOK_EPOCH: AtomicU64 = AtomicU64::new(1);
static ACTIVE_HOOK_EPOCH: AtomicU64 = AtomicU64::new(0);
static DRAIN_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static DROPPED_ROWS: AtomicU32 = AtomicU32::new(0);
static STATE: OnceLock<Mutex<VecDeque<ExactProvinceSupplyTooltipRow>>> = OnceLock::new();
static LIFECYCLE: Mutex<()> = Mutex::new(());

thread_local! {
    static OPEN: RefCell<Vec<OpenObservation>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy)]
struct OpenObservation {
    province_id: u32,
    observer_tag_db_id: u32,
    tick: i64,
    remaining_supply_raw: Option<i64>,
    duplicate_sink: bool,
}

/// How many yields a rollback spends waiting for in-flight callbacks to leave
/// before it gives up and RETAINS instead of freeing.
///
/// Bounded, because the alternative is this thread spinning forever on a
/// callback that is itself blocked. Exhausting it is not a failure: it selects
/// the conservative disposition, where the trampoline stays allocated for the
/// life of the process and nothing is freed underneath a thread.
const QUIESCE_YIELDS: u32 = 4096;

/// Take `target` down, and FREE its trampoline only if no callback can be
/// inside the detour.
///
/// # The defect this exists for
///
/// `inline_detour::remove` restores the target's bytes and then FREES the
/// trampoline. It refuses if a frozen thread is stopped in either of those, but
/// it cannot see a thread stopped inside the DETOUR BODY -- here, inside
/// `hook_formatter` before its call through `ORIGINAL_FORMATTER`. That thread
/// resumes and calls a trampoline the removal has freed.
///
/// The install-failure rollbacks used to remove immediately, so a formatter that
/// had been LIVE while the producer's enable was refused was freed with
/// callbacks possibly in flight. The full teardown drained first; the rollback
/// path did not. Returns true when the target was removed, false when it was
/// disabled and RETAINED.
unsafe fn take_down_freeing_only_when_quiet(target: usize) -> bool {
    // ADMISSION FIRST. Nothing new may enter the detour while this drains, or
    // the drain is measuring a set that keeps refilling.
    ADMISSION_OPEN.store(false, Ordering::Release);
    // THE DISABLE'S ANSWER IS READ. A refused disable leaves the detour LIVE,
    // and a live detour means callbacks keep arriving -- so an
    // `ACTIVE_CALLBACKS == 0` observed afterwards is not stable, and freeing on
    // it frees a trampoline a thread is about to call (review round 2, HIGH).
    // Only a target that is genuinely off may be freed.
    let off = matches!(
        unsafe { inline_detour::disable(target as *mut c_void) },
        Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED))
    );
    if !off {
        return false;
    }
    for _ in 0..QUIESCE_YIELDS {
        if ACTIVE_CALLBACKS.load(Ordering::Acquire) == 0 {
            break;
        }
        std::thread::yield_now();
    }
    if ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 {
        // RETAINED. The detour is off the target so nothing new can enter it,
        // and the trampoline stays allocated for whoever is still inside.
        return false;
    }
    unsafe { inline_detour::remove(target as *mut c_void) }.is_ok()
}

struct CallbackLease;
impl CallbackLease {
    fn enter() -> Self { ACTIVE_CALLBACKS.fetch_add(1, Ordering::AcqRel); Self }
}
impl Drop for CallbackLease {
    fn drop(&mut self) { ACTIVE_CALLBACKS.fetch_sub(1, Ordering::AcqRel); }
}

pub fn install() -> Result<(), String> {
    let _lock = LIFECYCLE.lock().map_err(|_| "supply tooltip hook lifecycle poisoned")?;
    if INSTALLED.load(Ordering::Acquire) { return Ok(()); }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 { return Err("supply tooltip hook module unavailable".into()); }
    let producer = base.checked_add(PRODUCER_RVA as usize).ok_or("producer address overflow")?;
    let formatter = base.checked_add(FORMATTER_RVA as usize).ok_or("formatter address overflow")?;
    verify_sentinel(producer, &PRODUCER_SENTINEL, "producer")?;
    verify_sentinel(formatter, &FORMATTER_SENTINEL, "formatter")?;
    verify_sentinel(
        base.checked_add(FORMATTER_CALL_RVA as usize).ok_or("callsite address overflow")?,
        &CALLSITE_SENTINEL,
        "formatter callsite",
    )?;
    MODULE_BASE.store(base, Ordering::Release);
    let producer_original = match unsafe {
        MinHook::create_hook(producer as *mut c_void, hook_producer as *mut c_void)
    } {
        Ok(value) => value,
        Err(status) => {
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("supply tooltip producer hook creation failed: {status}"));
        }
    };
    ORIGINAL_PRODUCER.store(producer_original as usize, Ordering::Release);
    let formatter_original = match unsafe {
        MinHook::create_hook(formatter as *mut c_void, hook_formatter as *mut c_void)
    } {
        Ok(value) => value,
        Err(status) => {
            let _ = unsafe { inline_detour::remove(producer as *mut c_void) };
            ORIGINAL_PRODUCER.store(0, Ordering::Release);
            MODULE_BASE.store(0, Ordering::Release);
            return Err(format!("supply tooltip formatter hook creation failed: {status}"));
        }
    };
    ORIGINAL_FORMATTER.store(formatter_original as usize, Ordering::Release);
    // Enable the nested sink before the outer producer so no opened trace can
    // run without its authenticated sink.
    if let Err(status) = unsafe { inline_detour::enable(formatter as *mut c_void) } {
        // Both go down the quiescent route: nothing is freed while a
        // thread may still be inside the detour body, which is the one
        // place `inline_detour::remove` cannot see.
        let _ = unsafe { take_down_freeing_only_when_quiet(formatter) };
        let _ = unsafe { take_down_freeing_only_when_quiet(producer) };
        ORIGINAL_PRODUCER.store(0, Ordering::Release);
        ORIGINAL_FORMATTER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("supply tooltip formatter hook enable failed: {status}"));
    }
    if let Err(status) = unsafe { inline_detour::enable(producer as *mut c_void) } {
        // THE FORMATTER WAS LIVE. A game thread can be inside
        // `hook_formatter` right now, between its entry and its call
        // through `ORIGINAL_FORMATTER`, and the trampoline this would
        // free is exactly what that call returns into.
        let _ = unsafe { take_down_freeing_only_when_quiet(formatter) };
        let _ = unsafe { take_down_freeing_only_when_quiet(producer) };
        ORIGINAL_PRODUCER.store(0, Ordering::Release);
        ORIGINAL_FORMATTER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("supply tooltip producer hook enable failed: {status}"));
    }
    let state = STATE.get_or_init(|| Mutex::new(VecDeque::new()));
    let Ok(mut queue) = state.lock() else {
        // BOTH WERE LIVE here, so both wait for their callbacks to leave before
        // anything is freed: a thread inside `hook_formatter` returns through a
        // trampoline this would otherwise have released underneath it.
        let _ = unsafe { take_down_freeing_only_when_quiet(producer) };
        let _ = unsafe { take_down_freeing_only_when_quiet(formatter) };
        ORIGINAL_PRODUCER.store(0, Ordering::Release);
        ORIGINAL_FORMATTER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err("supply tooltip hook queue poisoned".into());
    };
    queue.clear();
    drop(queue);
    DROPPED_ROWS.store(0, Ordering::Release);
    let hook_epoch = NEXT_HOOK_EPOCH.fetch_add(1, Ordering::AcqRel);
    if hook_epoch == 0 {
        // BOTH WERE LIVE here, so both wait for their callbacks to leave before
        // anything is freed: a thread inside `hook_formatter` returns through a
        // trampoline this would otherwise have released underneath it.
        let _ = unsafe { take_down_freeing_only_when_quiet(producer) };
        let _ = unsafe { take_down_freeing_only_when_quiet(formatter) };
        ORIGINAL_PRODUCER.store(0, Ordering::Release);
        ORIGINAL_FORMATTER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err("supply tooltip hook epoch exhausted".into());
    }
    ACTIVE_HOOK_EPOCH.store(hook_epoch, Ordering::Release);
    // ADMISSION OPENS LAST, after every piece of state a callback reads is
    // published. It closes first on the way out, which is what lets a teardown
    // drain a set that is emptying rather than one that keeps refilling.
    ADMISSION_OPEN.store(true, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn drain() -> Result<ExactProvinceSupplyTooltipBatch, String> {
    if !INSTALLED.load(Ordering::Acquire) {
        return Err("supply tooltip hooks are not installed".into());
    }
    let state = STATE.get().ok_or("supply tooltip hook state unavailable")?;
    let mut queue = state.lock().map_err(|_| "supply tooltip hook queue poisoned")?;
    let rows = queue.drain(..).collect();
    let drain_sequence = DRAIN_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if drain_sequence == 0 { return Err("supply tooltip drain sequence exhausted".into()); }
    Ok(ExactProvinceSupplyTooltipBatch {
        hook_epoch: ACTIVE_HOOK_EPOCH.load(Ordering::Acquire),
        drain_sequence,
        dropped_rows: DROPPED_ROWS.swap(0, Ordering::AcqRel),
        rows,
    })
}

pub fn uninstall() -> Result<(), String> {
    let _lock = LIFECYCLE.lock().map_err(|_| "supply tooltip hook lifecycle poisoned")?;
    if !INSTALLED.swap(false, Ordering::AcqRel) { return Ok(()); }
    // ADMISSION FIRST, THEN THE DRAIN, and the drain is BOUNDED.
    //
    // The loop this replaces waited for `ACTIVE_CALLBACKS == 0` with the
    // detours still LIVE, so new callbacks kept entering and the wait had no
    // bound at all; and even a zero it did observe was stale by the time the
    // disable ran (review round 2, HIGH). Closing admission first makes the set
    // one that empties. The remaining residency -- a thread already inside the
    // detour body -- is what `take_down_freeing_only_when_quiet` refuses to free
    // underneath.
    ADMISSION_OPEN.store(false, Ordering::Release);
    let base = MODULE_BASE.swap(0, Ordering::AcqRel);
    let targets = [base + PRODUCER_RVA as usize, base + FORMATTER_RVA as usize];
    let mut errors = Vec::new();
    for target in targets {
        if !unsafe { take_down_freeing_only_when_quiet(target) } {
            // RETAINED, and named. Nothing was freed, the detour is off or was
            // refused, and the trampoline and originals stay valid for the life
            // of the process.
            errors.push(format!(
                "{target:#x} retained: the detour could not be taken down and freed \
                 while callbacks may still be inside it"
            ));
        }
    }
    // THE ORIGINALS SURVIVE A RETAINED TEARDOWN. A callback still inside a
    // retained detour reads these to call through; zeroing them makes it take
    // the `== 0` branch and return a wrong answer instead of the engine's
    // (review round 2, HIGH). They are cleared only when nothing is retained.
    if errors.is_empty() {
        ORIGINAL_PRODUCER.store(0, Ordering::Release);
        ORIGINAL_FORMATTER.store(0, Ordering::Release);
    }
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

unsafe extern "system" fn hook_producer(province: usize, builder: usize) {
    let _lease = CallbackLease::enter();
    let original_addr = ORIGINAL_PRODUCER.load(Ordering::Acquire);
    if original_addr == 0 { return; }
    if !ADMISSION_OPEN.load(Ordering::Acquire) {
        // Admission is closed for a teardown. The engine is still waiting on
        // this call, so the original runs; nothing is observed.
        let original: ProducerFn = mem::transmute(original_addr);
        original(province, builder);
        return;
    }
    let original: ProducerFn = mem::transmute(original_addr);
    let opened = open_observation(province);
    if let Some(open) = opened { OPEN.with(|stack| stack.borrow_mut().push(open)); }
    original(province, builder);
    if opened.is_some() {
        let completed = OPEN.with(|stack| stack.borrow_mut().pop());
        if let Some(open) = completed {
            if !open.duplicate_sink {
                if let Some(remaining_supply_raw) = open.remaining_supply_raw {
                    publish(ExactProvinceSupplyTooltipRow {
                        province_id: open.province_id,
                        observer_tag_db_id: open.observer_tag_db_id,
                        remaining_supply_raw,
                        fixed_scale: FIXED_SCALE,
                        displayed_decimals: DISPLAY_DECIMALS,
                        tick: open.tick,
                        sequence: NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel),
                    });
                }
            }
        }
    }
}

unsafe extern "system" fn hook_formatter(dst: usize, raw: i64, decimals: u32) -> usize {
    let _lease = CallbackLease::enter();
    if ADMISSION_OPEN.load(Ordering::Acquire)
        && decimals == DISPLAY_DECIMALS && immediate_game_return_rva() == Some(FORMATTER_RETURN_RVA) {
        OPEN.with(|stack| {
            if let Some(open) = stack.borrow_mut().last_mut() {
                if open.remaining_supply_raw.replace(raw).is_some() { open.duplicate_sink = true; }
            }
        });
    }
    let original_addr = ORIGINAL_FORMATTER.load(Ordering::Acquire);
    if original_addr == 0 { return 0; }
    let original: FormatterFn = mem::transmute(original_addr);
    original(dst, raw, decimals)
}

fn open_observation(province: usize) -> Option<OpenObservation> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let province_id = read_u32(province.checked_add(CPROVINCE_ID_OFFSET)?).ok()?;
    // ATTRIBUTED. This returned a bare, uncounted `None` -- unlike the queue
    // overflow beside it, which increments DROPPED_ROWS -- so an out-of-space
    // province word left no trace at all. It files under the MAP unit rather
    // than the supply family's: this hook holds no live CSupply ceiling, it is
    // range-checking an identity against the id space's structural width.
    if province_id == 0
        || compact_contract::bound_trips::bound_exceeded_u32(
            province_id,
            MAX_PROVINCE_ID,
            compact_contract::bound_trips::BOUND_TRIP_MAP_PROVINCE_ID,
        )
    {
        return None;
    }
    let current_ref_fn: CurrentCountryRefFn = unsafe {
        mem::transmute(base.checked_add(CURRENT_COUNTRY_REF_RVA)?)
    };
    let current_ref = unsafe { current_ref_fn() };
    let raw_ref = read_i32(current_ref as usize).ok()?;
    let observer_tag_db_id = if raw_ref == 0 {
        0
    } else {
        let convert: CountryRefToIndexFn = unsafe {
            mem::transmute(base.checked_add(COUNTRY_REF_TO_INDEX_RVA)?)
        };
        unsafe { convert(current_ref) }
    };
    if observer_tag_db_id == 0 || observer_tag_db_id > MAX_OBSERVER_TAG_DB_ID { return None; }
    let tick = i64::from(read_i32(base.checked_add(CURRENT_TICK_RVA)?).ok()?);
    (tick >= 0).then_some(OpenObservation {
        province_id,
        observer_tag_db_id,
        tick,
        remaining_supply_raw: None,
        duplicate_sink: false,
    })
}

fn publish(row: ExactProvinceSupplyTooltipRow) {
    if row.sequence == 0 { DROPPED_ROWS.fetch_add(1, Ordering::AcqRel); return; }
    let Some(state) = STATE.get() else { DROPPED_ROWS.fetch_add(1, Ordering::AcqRel); return; };
    let Ok(mut queue) = state.lock() else { DROPPED_ROWS.fetch_add(1, Ordering::AcqRel); return; };
    if queue.len() == MAX_ROWS {
        queue.pop_front();
        DROPPED_ROWS.fetch_add(1, Ordering::AcqRel);
    }
    queue.push_back(row);
}

fn immediate_game_return_rva() -> Option<u32> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let pe_offset = read_u32(base.checked_add(0x3c)?).ok()? as usize;
    let image_size = read_u32(base.checked_add(pe_offset)?.checked_add(0x50)?).ok()? as usize;
    let mut stack = [ptr::null_mut::<c_void>(); 8];
    let count = unsafe {
        RtlCaptureStackBackTrace(0, stack.len() as u32, stack.as_mut_ptr(), ptr::null_mut())
    } as usize;
    let end = base.checked_add(image_size)?;
    stack[..count].iter().map(|p| *p as usize)
        .find(|address| *address >= base && *address < end)
        .and_then(|address| u32::try_from(address - base).ok())
}

fn verify_sentinel(address: usize, expected: &[u8], name: &str) -> Result<(), String> {
    let mut actual = vec![0u8; expected.len()];
    read_exact(address, &mut actual)?;
    if actual != expected { return Err(format!("supply tooltip {name} sentinel mismatch")); }
    Ok(())
}
fn read_u32(address: usize) -> Result<u32, String> { let mut b=[0u8;4]; read_exact(address,&mut b)?; Ok(u32::from_le_bytes(b)) }
fn read_i32(address: usize) -> Result<i32, String> { let mut b=[0u8;4]; read_exact(address,&mut b)?; Ok(i32::from_le_bytes(b)) }
fn read_exact(address: usize, out: &mut [u8]) -> Result<(), String> {
    if address == 0 { return Err("null read".into()); }
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(GetCurrentProcess(), address as *const c_void, out.as_mut_ptr().cast(), out.len(), &mut read)
    };
    if ok == 0 || read != out.len() { Err(format!("read failed at {address:#x}")) } else { Ok(()) }
}

/// THE DRAIN, DRIVEN, with a real callback really inside the real detour.
///
/// # What was missing
///
/// `take_down_freeing_only_when_quiet` is the worked example `inline_detour`'s
/// own `remove` contract points at: close admission, read the disable's answer,
/// compare the in-flight count against zero, and free ONLY then. Every part of
/// it is a response to a defect lane LI found -- a rollback that removed
/// immediately, a drain that measured a set still refilling, a zero observed
/// while the detour was still live. None of it had ever been executed: an
/// install refuses unless the running process is the exact-a729 `hoi4.exe`, so
/// the whole teardown was reachable only in a real game, and only on its
/// success path at that.
///
/// This drives it directly, on a synthetic target in this test binary, with
/// [`hook_formatter`] -- the PRODUCTION detour body -- as the detour and a
/// blocking stand-in as the original it calls through. A thread parked in there
/// is exactly the residency the drain exists to refuse to free underneath.
#[cfg(test)]
mod drain_tests {
    use super::*;
    use minhook::MinHookLock;
    use std::sync::atomic::AtomicBool as TestFlag;

    static GATE_OPEN: TestFlag = TestFlag::new(false);
    static ENTERED: TestFlag = TestFlag::new(false);

    /// The synthetic target. Its body never runs: the detour replaces it.
    #[inline(never)]
    extern "system" fn formatter_victim(_dst: usize, raw: i64, _decimals: u32) -> usize {
        raw as usize
    }

    /// The "original" [`hook_formatter`] calls through, which BLOCKS -- so the
    /// production body is genuinely resident when the drain runs.
    #[inline(never)]
    unsafe extern "system" fn blocking_original(_dst: usize, raw: i64, _decimals: u32) -> usize {
        ENTERED.store(true, Ordering::Release);
        while !GATE_OPEN.load(Ordering::Acquire) {
            // A bare spin: this thread is about to be suspended by the freeze
            // the disable takes.
            std::hint::spin_loop();
        }
        raw as usize
    }

    /// Spin until `ready`, or FAIL rather than spin for ever.
    ///
    /// The condition is one a REGRESSION can make unreachable: if the detour
    /// stops being physically installed while `enable` still reports success,
    /// the callback never enters and a bare loop hangs the test binary. A
    /// deadline turns that into a named failure instead of a wedged battery.
    fn wait_until(what: &str, ready: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !ready() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            std::thread::yield_now();
        }
    }

    /// Puts everything back however this test leaves, including during an
    /// unwind: the gate is opened so the parked thread can finish, the module's
    /// globals are restored, and -- ONCE THE DETOUR IS PROVABLY EMPTY -- the
    /// synthetic target is physically restored and its hook removed. If it
    /// cannot be proven empty the hook is retained and the fact is printed;
    /// removing it anyway would be the use-after-free under test.
    ///
    /// The first spelling restored only the two globals. A panic between the
    /// enable and the cleanup would then have left this test binary with one of
    /// its own functions permanently detoured to the production formatter, and
    /// a live entry in the process-global MinHook table for every later test to
    /// trip over.
    struct Restore {
        original_formatter: usize,
        admission: bool,
        target: *mut c_void,
    }

    impl Drop for Restore {
        fn drop(&mut self) {
            GATE_OPEN.store(true, Ordering::Release);
            // The parked thread must have LEFT the detour before the hook is
            // removed, or this cleanup is itself the use-after-free the test is
            // about. Bounded, and it gives up rather than blocking for ever.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0
                && std::time::Instant::now() < deadline
            {
                std::thread::yield_now();
            }
            if ACTIVE_CALLBACKS.load(Ordering::Acquire) == 0 {
                let _ = unsafe { inline_detour::disable(self.target) };
                let _ = unsafe { inline_detour::remove(self.target) };
            } else {
                // THE DETOUR STILL COMES OFF. Restoring the target's bytes
                // frees nothing and strands nobody -- a thread inside the body
                // returns through the trampoline, which a disable leaves alone
                // -- so nothing new can enter even on this path.
                let _ = unsafe { inline_detour::disable(self.target) };
                // NOTHING IS FREED, and it is SAID. A leaked hook on a
                // synthetic test target is recoverable; a trampoline freed
                // under a thread still inside the detour is not, and it is the
                // exact fault this test is about. Reported rather than
                // panicked: a panic during an unwind aborts the process and
                // destroys the report of whatever failed first. The process is
                // this test's own child, so what it leaks dies with it.
                eprintln!(
                    "supply drain test: a callback never left the detour; the hook \
                     on {:p} is deliberately RETAINED rather than freed under it",
                    self.target
                );
            }
            ORIGINAL_FORMATTER.store(self.original_formatter, Ordering::Release);
            ADMISSION_OPEN.store(self.admission, Ordering::Release);
        }
    }

    /// The env var that marks the child process, so the parent half returns
    /// instead of recursing.
    const CHILD_MARKER: &str = "SUPPLY_DRAIN_TEST_CHILD";

    /// Run the `#[ignore]`d child of this pair in a FRESH PROCESS, and fail if
    /// it does not pass.
    ///
    /// # Why this test is contained
    ///
    /// Its failure mode IS the regression it reports: a callback that never
    /// leaves the detour. When that happens the parked thread cannot be joined
    /// and the hook cannot be freed without performing the very use-after-free
    /// under test, so the honest cleanup is to retain -- and a retained hook
    /// plus a looping thread in a SHARED test binary make every test that runs
    /// afterwards untrustworthy. In a process of its own, what the failure
    /// leaks dies with it.
    ///
    /// The parent bounds the wait, kills the child rather than hanging, and
    /// requires the child to have actually RUN: `libtest` exits 0 when a filter
    /// matches nothing, so a renamed child would otherwise leave this proof
    /// green while proving nothing.
    fn run_in_child(test_name: &str) {
        assert!(
            std::env::var(CHILD_MARKER).is_err(),
            "a child must never spawn a grandchild"
        );
        let exe = std::env::current_exe().expect("the test binary's own path");
        let mut child = std::process::Command::new(exe)
            .args([
                "--exact",
                test_name,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_MARKER, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the child test process starts");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        let status = loop {
            match child.try_wait().expect("the child is waitable") {
                Some(status) => break status,
                None if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{test_name} hung in its child process; the child was killed");
                }
                None => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        };
        let output = child.wait_with_output().unwrap_or_else(|_| std::process::Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            status.success(),
            "{test_name} failed in its child process:\n{stdout}\n{stderr}"
        );
        assert!(
            stdout.contains("1 passed") && stdout.contains("0 failed"),
            "{test_name} was filtered away rather than run:\n{stdout}"
        );
    }

    /// CONTAINED. See [`run_in_child`]: this test's own failure mode leaves a
    /// callback looping inside a detour that cannot then be safely freed, so it
    /// runs in a process of its own and the claims live on the child below.
    #[test]
    fn a_callback_in_flight_retains_the_trampoline_and_a_quiet_one_frees_it() {
        run_in_child(
            "a729_tooltip_hook::drain_tests::child_a_callback_in_flight_retains_the_trampoline_and_a_quiet_one_frees_it",
        );
    }

    /// A CALLBACK IN FLIGHT MAKES THE TEARDOWN RETAIN, and the trampoline is
    /// not freed; the same teardown frees once it has left.
    ///
    /// Three claims, and the first is the one LI's review round 2 was about:
    ///
    /// 1. THE LEASE IS TAKEN BEFORE ADMISSION IS TESTED. The callback enters
    ///    with admission ALREADY CLOSED and the drain still sees a non-zero
    ///    count -- so a teardown that closed admission and then assumed the
    ///    detour was empty would be wrong. Moving `CallbackLease::enter()`
    ///    below the admission check makes this count zero and the drain frees
    ///    under a live callback.
    /// 2. THE DRAIN REFUSES RATHER THAN FREEING. It returns false, having
    ///    disabled the target and left the hook and its trampoline in place,
    ///    and it did NOT call `remove` -- proven by the target still being
    ///    removable afterwards, which a freed hook would not be.
    /// 3. IT IS NOT SIMPLY ALWAYS REFUSING. With the callback gone, the same
    ///    call frees. A drain hard-wired to return false would pass 1 and 2.
    ///
    /// kills: deleting the in-flight comparison, or moving it above the
    /// disable, or taking the lease after the admission test.
    #[test]
    #[ignore = "contained: driven by a_callback_in_flight_retains_the_trampoline_and_a_quiet_one_frees_it"]
    fn child_a_callback_in_flight_retains_the_trampoline_and_a_quiet_one_frees_it() {
        assert!(
            std::env::var(CHILD_MARKER).is_ok(),
            "this test is only sound in the child process its parent spawns"
        );
        let target = formatter_victim as *mut c_void;
        let _ = unsafe { inline_detour::disable(target) };
        let _ = unsafe { inline_detour::remove(target) };
        GATE_OPEN.store(false, Ordering::Release);
        ENTERED.store(false, Ordering::Release);
        let _restore = Restore {
            original_formatter: ORIGINAL_FORMATTER.load(Ordering::Acquire),
            admission: ADMISSION_OPEN.load(Ordering::Acquire),
            target,
        };
        ORIGINAL_FORMATTER.store(blocking_original as usize, Ordering::Release);
        // CLOSED BEFORE THE CALLBACK ENTERS. Claim 1 rests on this: whatever
        // the count is when the drain reads it, it was not admission that put
        // it there.
        ADMISSION_OPEN.store(false, Ordering::Release);

        unsafe { MinHook::create_hook(target, hook_formatter as *mut c_void) }
            .expect("the synthetic formatter is hookable");
        unsafe { inline_detour::enable(target) }.expect("the production detour goes live");

        let parked = std::thread::spawn(move || {
            let victim: FormatterFn = unsafe { mem::transmute(formatter_victim as *mut c_void) };
            unsafe { victim(0, 42, DISPLAY_DECIMALS) }
        });
        wait_until("the production formatter body to enter the original", || {
            ENTERED.load(Ordering::Acquire)
        });
        assert_ne!(
            ACTIVE_CALLBACKS.load(Ordering::Acquire),
            0,
            "the production body must hold its lease with admission ALREADY \
             closed: a lease taken after the admission test would read zero here"
        );

        let retained = unsafe { take_down_freeing_only_when_quiet(target as usize) };
        assert!(
            !retained,
            "a teardown that can still have a callback inside the detour must \
             RETAIN rather than free"
        );
        assert!(
            !ADMISSION_OPEN.load(Ordering::Acquire),
            "the drain leaves admission closed"
        );
        // The detour is OFF -- so nothing new can enter -- and the hook is
        // still there, which is what "retained" means and what a `remove`
        // would have destroyed.
        let lock = MinHookLock::acquire().expect("the lock is free");
        assert_eq!(
            MinHook::hook_is_enabled(&lock, target),
            Ok(false),
            "the retained target must be disabled, not still live"
        );
        assert!(
            MinHook::patch_footprint(&lock, target).is_ok(),
            "the hook and its trampoline must still exist: a freed one answers \
             MH_ERROR_NOT_CREATED"
        );
        drop(lock);

        GATE_OPEN.store(true, Ordering::Release);
        // BOUNDED BEFORE THE JOIN, so a detour that stops returning fails this
        // test instead of hanging the binary it runs in.
        wait_until("the callback to leave the detour", || {
            ACTIVE_CALLBACKS.load(Ordering::Acquire) == 0
        });
        let answered = parked.join().expect("the parked callback returns");
        assert_eq!(answered, 42, "it returned through the original it was given");

        let freed = unsafe { take_down_freeing_only_when_quiet(target as usize) };
        assert!(
            freed,
            "with nothing in flight the same teardown frees; a drain that \
             always refused would have passed everything above"
        );
        let lock = MinHookLock::acquire().expect("the lock is free");
        assert_eq!(
            MinHook::patch_footprint(&lock, target),
            Err(MH_STATUS::MH_ERROR_NOT_CREATED),
            "the hook is gone, so its trampoline was freed"
        );
        drop(lock);
    }
}
