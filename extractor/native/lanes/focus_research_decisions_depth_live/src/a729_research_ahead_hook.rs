//! Exact-a729 observed producer for A29 research ahead-of-time substitutions.
//!
//! This is deliberately not a second implementation of the penalty formula.
//! The hook joins the active `CTechnology` selected by the UI producer to the
//! exact fixed-5 substitution records consumed by the localization engine.
//! Process pointers are retained only until the collector joins the receipt to
//! its same-hour active research-slot walk.

use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

pub const UI_YEAR_PRODUCER_RVA: usize = 0x01BC_EA60;
pub const EFFECTIVE_VALUE_PRODUCER_RVA: usize = 0x00AB_E4C0;
pub const LOCALIZATION_SUBSTITUTION_CONSUMER_RVA: usize = 0x0223_0F20;
pub const REDUCED_KEY_RVA: usize = 0x0292_DD80;
pub const UNREDUCED_KEY_RVA: usize = 0x0292_DDA0;
pub const VALUE_NAME_RVA: usize = 0x0272_4F3C;
pub const RED_NAME_RVA: usize = 0x0292_DD78;
pub const REDUCED_YEARS_NAME_RVA: usize = 0x0292_DD68;
pub const YEARS_NAME_RVA: usize = 0x0292_DD5C;

pub const EXACT_ROOT_JOB_ID: &str =
    "20260720T124311Z-w5-a29-effective-producer-roots-e50d79d0";
pub const EXACT_ROOT_MANIFEST_SHA256: &str =
    "82884a9a122f95298d5068bdb68590806346c2c1ac274ca3b56889bb0e5abcc2";
pub const SENTINEL_JOB_ID: &str =
    "20260720T130641Z-w5-a29-hook-sentinels-and-substitution-i-9016d012";
pub const SENTINEL_DUMP_SHA256: &str =
    "3fbca6726f09499e6cf1961969ae4154b9c265f8e5c688f8d939d8397387cee2";
pub const EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

const UI_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54,
    0x41,
];
const PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20,
    0x55,
];
const FORMATTER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x41, 0x56, 0x41, 0x57,
    0x48,
];
const TECHNOLOGY_TEMPLATE_OFFSET: usize = 0x160;
const TECHNOLOGY_REDUCTION_OFFSET: usize = 0x1c0;
const TECHNOLOGY_DEFINITION_INDEX_OFFSET: usize = 0x08;
const VIEW_TECH_STATUS_OFFSET: usize = 0x520;
const TECH_STATUS_ACTIVE_TECH_OFFSET: usize = 0x18;
const DATE_CURRENT_RAW_OFFSET: usize = 0x08;
const SUBSTITUTION_RECORD_STRIDE: usize = 0x68;
const SUBSTITUTION_TYPE_OFFSET: usize = 0;
const SUBSTITUTION_NAME_OFFSET: usize = 8;
const SUBSTITUTION_FIXED5_OFFSET: usize = 0x50;
const FIXED5_SUBSTITUTION_TYPE: u32 = 9;
const MAX_FIXED5: i64 = 10_000_000_000;
const MAX_QUEUE: usize = 4096;
const QUIESCE_TIMEOUT: Duration = Duration::from_secs(30);

type UiProducerFn = unsafe extern "system" fn(usize, usize, usize) -> usize;
type EffectiveProducerFn =
    unsafe extern "system" fn(usize, *mut i64, usize, usize, i64, i64) -> *mut i64;
type LocalizationFn = unsafe extern "system" fn(usize, usize, usize, u32) -> usize;

static ORIGINAL_UI: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_PRODUCER: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_FORMATTER: AtomicUsize = AtomicUsize::new(0);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static HOOK_SET_PRESENT: AtomicBool = AtomicBool::new(false);
static TEARDOWN_ACTIVE: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static QUEUE: OnceLock<Mutex<VecDeque<RawResearchAheadObservation>>> = OnceLock::new();
static FATAL: OnceLock<Mutex<Option<String>>> = OnceLock::new();

thread_local! {
    static UI_CONTEXT: RefCell<Option<UiContext>> = const { RefCell::new(None) };
    static PRODUCER_CONTEXT: RefCell<Option<ProducerContext>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug)]
struct UiContext {
    technology_address: usize,
}

#[derive(Clone, Debug)]
struct ProducerContext {
    technology_address: usize,
    template_address: usize,
    date_record_address: usize,
    applied_reduction_years_raw: i64,
    baseline_reduction_years_raw: i64,
    observed: Option<ObservedSubstitutions>,
    invalid_reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObservedSubstitutions {
    branch: u32,
    ahead_of_time_years_raw: i64,
    ahead_of_time_penalty_raw: i64,
    ahead_of_time_reduction_years_raw: i64,
    ahead_of_time_reduced_penalty_raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawResearchAheadObservation {
    pub callback_sequence: u64,
    pub callback_thread_id: u32,
    pub callback_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    pub game_hour_raw: i32,
    pub technology_address: usize,
    pub technology_template_address: usize,
    pub technology_definition_index_raw: u32,
    pub applied_reduction_years_raw: i64,
    pub baseline_reduction_years_raw: i64,
    pub ahead_of_time_years_raw: i64,
    pub ahead_of_time_penalty_raw: i64,
    pub ahead_of_time_reduction_years_raw: i64,
    pub ahead_of_time_reduced_penalty_raw: i64,
    pub localization_branch: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledAddresses {
    pub ui_year_producer: usize,
    pub effective_value_producer: usize,
    pub localization_substitution_consumer: usize,
}

struct CallbackGuard;
impl CallbackGuard {
    fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for CallbackGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

fn queue() -> &'static Mutex<VecDeque<RawResearchAheadObservation>> {
    QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn fatal() -> &'static Mutex<Option<String>> {
    FATAL.get_or_init(|| Mutex::new(None))
}

fn gate_open() -> bool {
    INSTALLED.load(Ordering::Acquire) && !TEARDOWN_ACTIVE.load(Ordering::Acquire)
}

pub fn installed() -> bool {
    gate_open()
}

pub fn drain() -> Result<Vec<RawResearchAheadObservation>, String> {
    if let Some(reason) = fatal().lock().map_err(|_| "A29 fatal mutex poisoned")?.clone() {
        return Err(reason);
    }
    let mut queue = queue().lock().map_err(|_| "A29 queue mutex poisoned")?;
    Ok(queue.drain(..).collect())
}

pub fn install() -> Result<InstalledAddresses, String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A29 install mutex poisoned")?;
    if gate_open() {
        return Ok(addresses(MODULE_BASE.load(Ordering::Acquire)));
    }
    if HOOK_SET_PRESENT.load(Ordering::Acquire) {
        return Err("A29 hook set is lifecycle-poisoned; uninstall before retry".into());
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    if base == 0 {
        return Err("A29 GetModuleHandleW(NULL) returned null".into());
    }
    let resolved = addresses(base);
    for (target, sentinel, label) in [
        (resolved.ui_year_producer, &UI_SENTINEL[..], "UI year producer"),
        (
            resolved.effective_value_producer,
            &PRODUCER_SENTINEL[..],
            "effective value producer",
        ),
        (
            resolved.localization_substitution_consumer,
            &FORMATTER_SENTINEL[..],
            "localization substitution consumer",
        ),
    ] {
        verify_sentinel(target, sentinel, label)?;
    }
    MODULE_BASE.store(base, Ordering::Release);
    TEARDOWN_ACTIVE.store(false, Ordering::Release);
    HOOK_SET_PRESENT.store(true, Ordering::Release);
    let specs = [
        (
            resolved.ui_year_producer,
            hook_ui_producer as *mut c_void,
            &ORIGINAL_UI,
        ),
        (
            resolved.effective_value_producer,
            hook_effective_producer as *mut c_void,
            &ORIGINAL_PRODUCER,
        ),
        (
            resolved.localization_substitution_consumer,
            hook_localization as *mut c_void,
            &ORIGINAL_FORMATTER,
        ),
    ];
    for (target, detour, original) in specs {
        match unsafe { MinHook::create_hook(target as *mut c_void, detour) } {
            Ok(trampoline) => original.store(trampoline as usize, Ordering::Release),
            Err(status) => {
                return rollback_install(
                    resolved,
                    format!("A29 hook create failed at {target:#x}: {status}"),
                )
            }
        }
    }
    for target in targets(resolved) {
        if let Err(status) = unsafe { MinHook::enable_hook(target as *mut c_void) } {
            return rollback_install(
                resolved,
                format!("A29 hook enable failed at {target:#x}: {status}"),
            );
        }
    }
    queue().lock().map_err(|_| "A29 queue mutex poisoned")?.clear();
    *fatal().lock().map_err(|_| "A29 fatal mutex poisoned")? = None;
    INSTALLED.store(true, Ordering::Release);
    Ok(resolved)
}

pub fn uninstall() -> Result<(), String> {
    let _guard = INSTALL_LOCK.lock().map_err(|_| "A29 install mutex poisoned")?;
    if !HOOK_SET_PRESENT.load(Ordering::Acquire) {
        return Ok(());
    }
    TEARDOWN_ACTIVE.store(true, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    let resolved = addresses(MODULE_BASE.load(Ordering::Acquire));
    disable_targets(resolved)?;
    let deadline = Instant::now() + QUIESCE_TIMEOUT;
    while IN_FLIGHT.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let remaining = IN_FLIGHT.load(Ordering::Acquire);
    if remaining != 0 {
        return Err(format!("A29 teardown has {remaining} in-flight callbacks"));
    }
    remove_targets(resolved)?;
    ORIGINAL_UI.store(0, Ordering::Release);
    ORIGINAL_PRODUCER.store(0, Ordering::Release);
    ORIGINAL_FORMATTER.store(0, Ordering::Release);
    MODULE_BASE.store(0, Ordering::Release);
    HOOK_SET_PRESENT.store(false, Ordering::Release);
    TEARDOWN_ACTIVE.store(false, Ordering::Release);
    Ok(())
}

fn rollback_install(
    resolved: InstalledAddresses,
    reason: String,
) -> Result<InstalledAddresses, String> {
    TEARDOWN_ACTIVE.store(true, Ordering::Release);
    INSTALLED.store(false, Ordering::Release);
    let rollback = disable_targets(resolved).and_then(|()| {
        let deadline = Instant::now() + QUIESCE_TIMEOUT;
        while IN_FLIGHT.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let remaining = IN_FLIGHT.load(Ordering::Acquire);
        if remaining != 0 {
            Err(format!("rollback has {remaining} in-flight callbacks"))
        } else {
            remove_targets(resolved)
        }
    });
    if rollback.is_ok() {
        ORIGINAL_UI.store(0, Ordering::Release);
        ORIGINAL_PRODUCER.store(0, Ordering::Release);
        ORIGINAL_FORMATTER.store(0, Ordering::Release);
        MODULE_BASE.store(0, Ordering::Release);
        HOOK_SET_PRESENT.store(false, Ordering::Release);
        TEARDOWN_ACTIVE.store(false, Ordering::Release);
    }
    Err(match rollback {
        Ok(()) => reason,
        Err(error) => format!("{reason}; rollback anomaly: {error}"),
    })
}

fn disable_targets(resolved: InstalledAddresses) -> Result<(), String> {
    let mut errors = Vec::new();
    for target in targets(resolved) {
        match unsafe { MinHook::disable_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_DISABLED) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("disable {target:#x}: {status}")),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn remove_targets(resolved: InstalledAddresses) -> Result<(), String> {
    let mut errors = Vec::new();
    for target in targets(resolved) {
        match unsafe { MinHook::remove_hook(target as *mut c_void) } {
            Ok(()) | Err(MH_STATUS::MH_ERROR_NOT_CREATED) => {}
            Err(status) => errors.push(format!("remove {target:#x}: {status}")),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn addresses(base: usize) -> InstalledAddresses {
    InstalledAddresses {
        ui_year_producer: base + UI_YEAR_PRODUCER_RVA,
        effective_value_producer: base + EFFECTIVE_VALUE_PRODUCER_RVA,
        localization_substitution_consumer: base + LOCALIZATION_SUBSTITUTION_CONSUMER_RVA,
    }
}

fn targets(value: InstalledAddresses) -> [usize; 3] {
    [
        value.ui_year_producer,
        value.effective_value_producer,
        value.localization_substitution_consumer,
    ]
}

unsafe extern "system" fn hook_ui_producer(view: usize, field: usize, output: usize) -> usize {
    let _guard = CallbackGuard::enter();
    let original = ORIGINAL_UI.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    let original: UiProducerFn = unsafe { mem::transmute(original) };
    let status = view
        .checked_add(VIEW_TECH_STATUS_OFFSET)
        .and_then(read_process_value::<usize>)
        .unwrap_or(0);
    let technology_address = status
        .checked_add(TECH_STATUS_ACTIVE_TECH_OFFSET)
        .and_then(read_process_value::<usize>)
        .unwrap_or(0);
    let previous = UI_CONTEXT.with(|slot| {
        slot.replace(Some(UiContext {
            technology_address,
        }))
    });
    let result = unsafe { original(view, field, output) };
    UI_CONTEXT.with(|slot| {
        slot.replace(previous);
    });
    result
}

unsafe extern "system" fn hook_effective_producer(
    template: usize,
    output_penalty: *mut i64,
    date_record: usize,
    localized_output: usize,
    applied_reduction: i64,
    baseline_reduction: i64,
) -> *mut i64 {
    let _guard = CallbackGuard::enter();
    let original = ORIGINAL_PRODUCER.load(Ordering::Acquire);
    if original == 0 {
        return output_penalty;
    }
    let original: EffectiveProducerFn = unsafe { mem::transmute(original) };
    let ui = UI_CONTEXT.with(|slot| *slot.borrow());
    let mut context = ProducerContext {
        technology_address: ui.map(|value| value.technology_address).unwrap_or(0),
        template_address: template,
        date_record_address: date_record,
        applied_reduction_years_raw: applied_reduction,
        baseline_reduction_years_raw: baseline_reduction,
        observed: None,
        invalid_reason: None,
    };
    if context.technology_address == 0
        || template == 0
        || date_record == 0
        || output_penalty.is_null()
        || localized_output == 0
    {
        context.invalid_reason = Some("A29 producer lacks its exact UI identity/output context");
    } else {
        let observed_template = context
            .technology_address
            .checked_add(TECHNOLOGY_TEMPLATE_OFFSET)
            .and_then(read_process_value::<usize>);
        let observed_reduction = context
            .technology_address
            .checked_add(TECHNOLOGY_REDUCTION_OFFSET)
            .and_then(read_process_value::<i64>);
        if observed_template != Some(template) || observed_reduction != Some(applied_reduction) {
            context.invalid_reason =
                Some("A29 producer arguments differ from the selected active technology");
        }
    }
    let previous = PRODUCER_CONTEXT.with(|slot| slot.replace(Some(context)));
    let result = unsafe {
        original(
            template,
            output_penalty,
            date_record,
            localized_output,
            applied_reduction,
            baseline_reduction,
        )
    };
    let completed = PRODUCER_CONTEXT.with(|slot| slot.replace(previous));
    if gate_open() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            if let Some(context) = completed {
                publish_completed(context, output_penalty);
            }
        }));
    }
    result
}

unsafe extern "system" fn hook_localization(
    output: usize,
    key: usize,
    arguments: usize,
    count: u32,
) -> usize {
    let _guard = CallbackGuard::enter();
    if gate_open() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            observe_substitutions(key, arguments, count);
        }));
    }
    let original = ORIGINAL_FORMATTER.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    let original: LocalizationFn = unsafe { mem::transmute(original) };
    unsafe { original(output, key, arguments, count) }
}

fn observe_substitutions(key: usize, arguments: usize, count: u32) {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 || arguments == 0 {
        return;
    }
    let branch = if key == base + REDUCED_KEY_RVA && count == 4 {
        2
    } else if key == base + UNREDUCED_KEY_RVA && count == 2 {
        1
    } else {
        return;
    };
    PRODUCER_CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(context) = slot.as_mut() else {
            return;
        };
        if context.observed.is_some() {
            context.invalid_reason = Some("A29 producer emitted more than one penalty branch");
            return;
        }
        let expected_names: &[usize] = if branch == 2 {
            &[
                VALUE_NAME_RVA,
                RED_NAME_RVA,
                REDUCED_YEARS_NAME_RVA,
                YEARS_NAME_RVA,
            ]
        } else {
            &[VALUE_NAME_RVA, YEARS_NAME_RVA]
        };
        let mut values = [0i64; 4];
        for (index, expected_name) in expected_names.iter().enumerate() {
            let record = arguments + index * SUBSTITUTION_RECORD_STRIDE;
            let kind = record
                .checked_add(SUBSTITUTION_TYPE_OFFSET)
                .and_then(read_process_value::<u32>);
            let name = record
                .checked_add(SUBSTITUTION_NAME_OFFSET)
                .and_then(read_process_value::<usize>);
            let value = record
                .checked_add(SUBSTITUTION_FIXED5_OFFSET)
                .and_then(read_process_value::<i64>);
            if kind != Some(FIXED5_SUBSTITUTION_TYPE)
                || name != base.checked_add(*expected_name)
                || !value.is_some_and(|value| (0..=MAX_FIXED5).contains(&value))
            {
                context.invalid_reason =
                    Some("A29 localization substitution record failed exact identity/domain");
                return;
            }
            values[index] = value.expect("checked above");
        }
        context.observed = Some(if branch == 2 {
            ObservedSubstitutions {
                branch,
                ahead_of_time_years_raw: values[3],
                ahead_of_time_penalty_raw: values[0],
                ahead_of_time_reduction_years_raw: values[2],
                ahead_of_time_reduced_penalty_raw: values[1],
            }
        } else {
            ObservedSubstitutions {
                branch,
                ahead_of_time_years_raw: values[1],
                ahead_of_time_penalty_raw: values[0],
                ahead_of_time_reduction_years_raw: 0,
                ahead_of_time_reduced_penalty_raw: values[0],
            }
        });
    });
}

fn publish_completed(context: ProducerContext, output_penalty: *mut i64) {
    if let Some(reason) = context.invalid_reason {
        poison(reason);
        return;
    }
    let output_value = read_process_value::<i64>(output_penalty as usize);
    let observed = match context.observed {
        Some(observed) => observed,
        None if output_value == Some(0) => {
            // Exact 0x140ABE4C0 no-display branch: the producer writes zero
            // and emits no substitution array; 0x141BCEA60 consumes the empty
            // localized output as NO_RESEARCH_PENALTY. Preserve the producer's
            // observed aggregate argument, but manufacture no display value.
            ObservedSubstitutions {
                branch: 0,
                ahead_of_time_years_raw: 0,
                ahead_of_time_penalty_raw: 0,
                ahead_of_time_reduction_years_raw: context.applied_reduction_years_raw,
                ahead_of_time_reduced_penalty_raw: 0,
            }
        }
        None => {
            poison(
                "A29 nonzero effective producer returned without observed localization substitutions",
            );
            return;
        }
    };
    let game_hour_raw = context
        .date_record_address
        .checked_add(DATE_CURRENT_RAW_OFFSET)
        .and_then(read_process_value::<i32>);
    let definition_index = context
        .technology_address
        .checked_add(TECHNOLOGY_DEFINITION_INDEX_OFFSET)
        .and_then(read_process_value::<u32>);
    if output_value != Some(observed.ahead_of_time_penalty_raw)
        || context.applied_reduction_years_raw != observed.ahead_of_time_reduction_years_raw
        || context.baseline_reduction_years_raw != 0
        || !game_hour_raw.is_some_and(|value| value > 0)
        || definition_index.is_none()
    {
        poison("A29 observed output/aggregate/date disagrees with the exact producer transaction");
        return;
    }
    let Some((ticks, frequency)) = qpc_now() else {
        poison("A29 callback QPC read failed");
        return;
    };
    let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if sequence == 0 {
        poison("A29 callback sequence overflowed");
        return;
    }
    let row = RawResearchAheadObservation {
        callback_sequence: sequence,
        callback_thread_id: unsafe { GetCurrentThreadId() },
        callback_qpc_ticks: ticks,
        qpc_frequency_hz: frequency,
        game_hour_raw: game_hour_raw.expect("checked above"),
        technology_address: context.technology_address,
        technology_template_address: context.template_address,
        technology_definition_index_raw: definition_index.expect("checked above"),
        applied_reduction_years_raw: context.applied_reduction_years_raw,
        baseline_reduction_years_raw: context.baseline_reduction_years_raw,
        ahead_of_time_years_raw: observed.ahead_of_time_years_raw,
        ahead_of_time_penalty_raw: observed.ahead_of_time_penalty_raw,
        ahead_of_time_reduction_years_raw: observed.ahead_of_time_reduction_years_raw,
        ahead_of_time_reduced_penalty_raw: observed.ahead_of_time_reduced_penalty_raw,
        localization_branch: observed.branch,
    };
    let mut queue = match queue().lock() {
        Ok(value) => value,
        Err(_) => {
            poison("A29 queue mutex poisoned");
            return;
        }
    };
    if queue.len() >= MAX_QUEUE {
        drop(queue);
        poison("A29 observation queue overflowed");
        return;
    }
    queue.push_back(row);
}

fn poison(reason: &str) {
    if let Ok(mut fatal) = fatal().lock() {
        if fatal.is_none() {
            *fatal = Some(reason.to_owned());
        }
    }
}

fn qpc_now() -> Option<(u64, u64)> {
    let mut ticks = 0i64;
    let mut frequency = 0i64;
    if unsafe { QueryPerformanceCounter(&mut ticks) } == 0
        || unsafe { QueryPerformanceFrequency(&mut frequency) } == 0
        || ticks <= 0
        || frequency <= 0
    {
        None
    } else {
        Some((ticks as u64, frequency as u64))
    }
}

fn verify_sentinel(address: usize, expected: &[u8], label: &str) -> Result<(), String> {
    let mut actual = vec![0u8; expected.len()];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            actual.as_mut_ptr().cast(),
            actual.len(),
            &mut read,
        )
    };
    if ok == 0 || read != actual.len() || actual != expected {
        return Err(format!(
            "A29 exact-a729 {label} sentinel mismatch at {address:#x}"
        ));
    }
    Ok(())
}

fn read_process_value<T: Copy>(address: usize) -> Option<T> {
    if address == 0 {
        return None;
    }
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
    };
    (ok != 0 && read == mem::size_of::<T>()).then(|| unsafe { value.assume_init() })
}
