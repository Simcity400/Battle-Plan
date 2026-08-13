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
        let _ = unsafe { inline_detour::remove(formatter as *mut c_void) };
        let _ = unsafe { inline_detour::remove(producer as *mut c_void) };
        ORIGINAL_PRODUCER.store(0, Ordering::Release);
        ORIGINAL_FORMATTER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("supply tooltip formatter hook enable failed: {status}"));
    }
    if let Err(status) = unsafe { inline_detour::enable(producer as *mut c_void) } {
        let _ = unsafe { inline_detour::disable(formatter as *mut c_void) };
        let _ = unsafe { inline_detour::remove(formatter as *mut c_void) };
        let _ = unsafe { inline_detour::remove(producer as *mut c_void) };
        ORIGINAL_PRODUCER.store(0, Ordering::Release);
        ORIGINAL_FORMATTER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err(format!("supply tooltip producer hook enable failed: {status}"));
    }
    let state = STATE.get_or_init(|| Mutex::new(VecDeque::new()));
    let Ok(mut queue) = state.lock() else {
        let _ = unsafe { inline_detour::disable(producer as *mut c_void) };
        let _ = unsafe { inline_detour::disable(formatter as *mut c_void) };
        let _ = unsafe { inline_detour::remove(producer as *mut c_void) };
        let _ = unsafe { inline_detour::remove(formatter as *mut c_void) };
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
        let _ = unsafe { inline_detour::disable(producer as *mut c_void) };
        let _ = unsafe { inline_detour::disable(formatter as *mut c_void) };
        let _ = unsafe { inline_detour::remove(producer as *mut c_void) };
        let _ = unsafe { inline_detour::remove(formatter as *mut c_void) };
        ORIGINAL_PRODUCER.store(0, Ordering::Release);
        ORIGINAL_FORMATTER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        return Err("supply tooltip hook epoch exhausted".into());
    }
    ACTIVE_HOOK_EPOCH.store(hook_epoch, Ordering::Release);
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
    while ACTIVE_CALLBACKS.load(Ordering::Acquire) != 0 { std::thread::yield_now(); }
    let base = MODULE_BASE.swap(0, Ordering::AcqRel);
    let targets = [base + PRODUCER_RVA as usize, base + FORMATTER_RVA as usize];
    let mut errors = Vec::new();
    for target in targets {
        match unsafe { inline_detour::disable(target as *mut c_void) } {
            Ok(())
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED))
            | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => errors.push(format!("disable {target:#x}: {status}")),
        }
        match unsafe { inline_detour::remove(target as *mut c_void) } {
            Ok(()) | Err(InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)) => {}
            Err(status) => errors.push(format!("remove {target:#x}: {status}")),
        }
    }
    ORIGINAL_PRODUCER.store(0, Ordering::Release);
    ORIGINAL_FORMATTER.store(0, Ordering::Release);
    ACTIVE_HOOK_EPOCH.store(0, Ordering::Release);
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

unsafe extern "system" fn hook_producer(province: usize, builder: usize) {
    let _lease = CallbackLease::enter();
    let original_addr = ORIGINAL_PRODUCER.load(Ordering::Acquire);
    if original_addr == 0 { return; }
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
    if decimals == DISPLAY_DECIMALS && immediate_game_return_rva() == Some(FORMATTER_RETURN_RVA) {
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
